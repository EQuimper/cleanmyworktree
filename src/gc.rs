//! `cmw rm`, `cmw gc`, `cmw clean-artifacts`.
//!
//! Safety rules, in order of importance:
//!   1. never touch the main checkout;
//!   2. never delete uncommitted work, or unpushed-and-unmerged commits, without `--force`;
//!   3. a branch is only deleted when it is positively known to be merged;
//!   4. `clean-artifacts` only deletes git-ignored paths.
use crate::cli::{CleanArtifactsArgs, GcArgs, RmArgs};
use crate::git::{self, Worktree};
use crate::inventory::{self, GhCache, RepoRef, StatusOpts, WorktreeStatus};
use crate::output::{CmwError, Ctx, Report, Result, human_bytes, sh_quote};
use crate::{config, lock, paths, rules};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const LOCK_TIMEOUT: Duration = Duration::from_secs(30);

/// Filesystem roots, injectable so tests never touch the real home directory.
struct Env {
    use_gh: bool,
    dd_root: PathBuf,
    trash: PathBuf,
    wt_root: PathBuf,
    scan_roots: Vec<PathBuf>,
}

impl Env {
    fn real() -> Self {
        Self {
            use_gh: true,
            dd_root: paths::home().join("Library/Developer/Xcode/DerivedData"),
            trash: paths::cmw_home().join("trash"),
            wt_root: paths::worktrees_root(),
            scan_roots: paths::scan_roots(),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Small filesystem helpers
// ---------------------------------------------------------------------------------------------

fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

fn delete_path(p: &Path) -> std::io::Result<()> {
    if std::fs::symlink_metadata(p)?.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) }
}

enum Evicted {
    Trashed(PathBuf),
    Deleted,
}

static TRASH_SEQ: AtomicU64 = AtomicU64::new(0);

/// Make `path` disappear from its location immediately: rename it into the trash dir (O(1) on the
/// same volume), or delete in place when the rename is impossible (e.g. other volume).
fn evict(path: &Path, trash: &Path) -> Result<Evicted> {
    let _ = std::fs::create_dir_all(trash);
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("dir");
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dest = trash.join(format!("{}-{}-{}-{}", std::process::id(), nanos, TRASH_SEQ.fetch_add(1, Ordering::Relaxed), name));
    match std::fs::rename(path, &dest) {
        Ok(()) => Ok(Evicted::Trashed(dest)),
        Err(_) => {
            std::fs::remove_dir_all(path).map_err(|e| CmwError::error(format!("could not delete {}: {e}", path.display())))?;
            Ok(Evicted::Deleted)
        }
    }
}

/// Finish an eviction: delete the trashed dir synchronously, or hand it to a detached `rm -rf`.
fn finish_evict(e: Evicted, sync: bool) -> Result<()> {
    let Evicted::Trashed(dir) = e else { return Ok(()) };
    if !sync {
        use std::os::unix::process::CommandExt;
        let spawned = Command::new("rm")
            .args(["-rf", "--"])
            .arg(&dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn();
        if spawned.is_ok() {
            return Ok(());
        }
    }
    std::fs::remove_dir_all(&dir).map_err(|e| CmwError::error(format!("could not delete {}: {e}", dir.display())))
}

/// Free bytes on the volumes holding `probe_paths`, to measure what a deletion really freed
/// (APFS clones make apparent sizes meaningless).
struct FreeProbe(Vec<(u64, PathBuf, u64)>);

impl FreeProbe {
    fn avail(path: &Path) -> Option<(u64, u64)> {
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::MetadataExt;
        let dev = std::fs::metadata(path).ok()?.dev();
        let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
        // SAFETY: `c` is a valid NUL-terminated path and `s` is a properly sized, zeroed out-parameter.
        let s = unsafe {
            let mut s: libc::statvfs = std::mem::zeroed();
            if libc::statvfs(c.as_ptr(), &mut s) != 0 {
                return None;
            }
            s
        };
        Some((dev, (s.f_bavail as u64).saturating_mul(s.f_frsize as u64)))
    }

    fn new<'a>(paths: impl IntoIterator<Item = &'a Path>) -> Self {
        let mut v: Vec<(u64, PathBuf, u64)> = Vec::new();
        for p in paths {
            if let Some((dev, free)) = Self::avail(p)
                && !v.iter().any(|(d, _, _)| *d == dev) {
                    v.push((dev, p.to_path_buf(), free));
                }
        }
        Self(v)
    }

    fn freed(&self) -> u64 {
        self.0.iter().filter_map(|(_, p, before)| Self::avail(p).map(|(_, after)| after.saturating_sub(*before))).sum()
    }
}

// ---------------------------------------------------------------------------------------------
// Xcode DerivedData
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct DdEntry {
    dir: PathBuf,
    workspace: Option<PathBuf>,
}

fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

fn workspace_path(plist: &Path) -> Option<PathBuf> {
    let via_plutil = Command::new("plutil")
        .args(["-extract", "WorkspacePath", "raw", "-o", "-"])
        .arg(plist)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());
    if let Some(s) = via_plutil {
        return Some(PathBuf::from(s));
    }
    // Fallback for hosts without plutil: plain XML plists.
    let text = std::fs::read_to_string(plist).ok()?;
    let key = "<key>WorkspacePath</key>";
    let rest = &text[text.find(key)? + key.len()..];
    let start = rest.find("<string>")? + "<string>".len();
    let end = rest[start..].find("</string>")?;
    let v = xml_unescape(rest[start..start + end].trim());
    (!v.is_empty()).then(|| PathBuf::from(v))
}

fn scan_derived_data(root: &Path) -> Vec<DdEntry> {
    let Ok(rd) = std::fs::read_dir(root) else { return Vec::new() };
    let mut out: Vec<DdEntry> = rd
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| {
            let plist = e.path().join("info.plist");
            plist.is_file().then(|| DdEntry { dir: e.path(), workspace: workspace_path(&plist) })
        })
        .collect();
    out.sort_by(|a, b| a.dir.cmp(&b.dir));
    out
}

/// DerivedData folders whose workspace lives inside `wt_path`.
fn dd_for_worktree(entries: &[DdEntry], wt_path: &Path) -> Vec<PathBuf> {
    let canon = canonical(wt_path);
    entries
        .iter()
        .filter(|e| e.workspace.as_ref().is_some_and(|w| w.starts_with(wt_path) || w.starts_with(&canon)))
        .map(|e| e.dir.clone())
        .collect()
}

/// DerivedData folders whose workspace no longer exists.
fn dd_orphans(entries: &[DdEntry]) -> Vec<PathBuf> {
    entries.iter().filter(|e| e.workspace.as_ref().is_some_and(|w| !w.exists())).map(|e| e.dir.clone()).collect()
}

// ---------------------------------------------------------------------------------------------
// Worktree removal (shared by rm and gc)
// ---------------------------------------------------------------------------------------------

fn repo_cwd(wts: &[Worktree], common: &Path) -> PathBuf {
    wts.iter().find(|w| w.is_main && !w.bare && w.path.is_dir()).map(|w| w.path.clone()).unwrap_or_else(|| common.to_path_buf())
}

struct RemoveOpts {
    dry_run: bool,
    sync: bool,
    keep_branch: bool,
}

struct Removal {
    derived_data: Vec<PathBuf>,
    watchman: bool,
    branch_deleted: bool,
    branch_kept_reason: Option<String>,
    /// "removed" | "deferred" (registration left for `git worktree prune`) | "not_needed" (dry run).
    admin_cleanup: &'static str,
}

fn watchman_available() -> bool {
    paths::find_in_path("watchman").is_some()
}

/// The `.git/worktrees/<name>` admin dir that belongs to exactly this worktree path, or None when it
/// cannot be proven. Existing worktree: follow its `.git` file. Missing directory: find the admin dir
/// whose `gitdir` file points at `<path>/.git`. Either way the admin dir's `gitdir` must point back.
fn resolve_admin(repo_cwd: &Path, wt_path: &Path) -> Option<PathBuf> {
    let common = canonical(&git::common_dir(repo_cwd).ok()?);
    let worktrees = common.join("worktrees");
    let target = canonical(wt_path).join(".git");
    let points_back = |admin: &Path| {
        std::fs::read_to_string(admin.join("gitdir")).is_ok_and(|t| {
            let g = PathBuf::from(t.trim());
            g == target || canonical(&g) == target || g == wt_path.join(".git")
        })
    };
    let candidate = if wt_path.is_dir() {
        let text = std::fs::read_to_string(wt_path.join(".git")).ok()?;
        let g = PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
        Some(if g.is_absolute() { g } else { wt_path.join(g) })
    } else {
        std::fs::read_dir(&worktrees).ok()?.flatten().map(|e| e.path()).find(|p| points_back(p))
    }?;
    let admin = canonical(&candidate);
    (admin.parent() == Some(worktrees.as_path()) && points_back(&admin) && !admin.join("locked").exists()).then_some(admin)
}

/// Delete exactly one resolved admin dir. Never runs a repo-wide `git worktree prune`.
fn drop_admin_dir(ctx: &Ctx, admin: Option<PathBuf>, wt_path: &Path) -> &'static str {
    if let Some(a) = admin
        && std::fs::remove_dir_all(&a).is_ok()
    {
        return "removed";
    }
    ctx.warn(format!(
        "could not identify the git registration of {}; it stays registered (inspect with `git worktree list`)",
        wt_path.display()
    ));
    "deferred"
}

fn remove_worktree(ctx: &Ctx, repo_cwd: &Path, st: &WorktreeStatus, dd: &[DdEntry], trash: &Path, o: &RemoveOpts) -> Result<Removal> {
    let derived_data = dd_for_worktree(dd, &st.path);
    let (delete_branch, kept) = match &st.branch {
        None => (false, Some("detached HEAD: there is no branch".to_string())),
        Some(_) if o.keep_branch => (false, Some("--keep-branch".to_string())),
        Some(_) if st.merged != Some(true) => {
            (false, Some(format!("not known to be merged ({}); its commits stay recoverable on the branch", st.merge_reason)))
        }
        Some(_) => (true, None),
    };
    let exists = st.path.is_dir();
    if o.dry_run {
        return Ok(Removal {
            derived_data,
            watchman: exists && watchman_available(),
            branch_deleted: delete_branch,
            branch_kept_reason: kept,
            admin_cleanup: "not_needed",
        });
    }

    // (a) external DerivedData
    for d in &derived_data {
        if let Err(e) = delete_path(d) {
            ctx.warn(format!("could not delete DerivedData {}: {e}", d.display()));
        }
    }
    // (b) watchman
    let mut watchman = false;
    if exists && watchman_available() {
        watchman = Command::new("watchman")
            .args(["watch-del"])
            .arg(&st.path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
    }
    // (c) the directory and its git registration
    // Resolve the admin dir BEFORE moving the directory; delete only that one afterwards.
    let admin = resolve_admin(repo_cwd, &st.path);
    let admin_cleanup;
    if exists {
        let evicted = evict(&st.path, trash)?;
        admin_cleanup = drop_admin_dir(ctx, admin, &st.path);
        finish_evict(evicted, o.sync)?;
    } else {
        admin_cleanup = drop_admin_dir(ctx, admin, &st.path);
    }
    // (d) branch
    let mut branch_deleted = false;
    let mut branch_kept_reason = kept;
    if delete_branch {
        let b = st.branch.as_deref().unwrap_or_default();
        match git::run(repo_cwd, &["branch", "-D", b]) {
            Ok(_) => branch_deleted = true,
            Err(e) => {
                ctx.warn(format!("worktree removed but branch {b} was kept: {}", e.message));
                branch_kept_reason = Some(e.message);
            }
        }
    }
    Ok(Removal { derived_data, watchman, branch_deleted, branch_kept_reason, admin_cleanup })
}

// ---------------------------------------------------------------------------------------------
// rm
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct RmReport {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub dry_run: bool,
    /// With `dry_run`, the `removed`, `branch_deleted` and `derived_data_removed` fields describe the plan.
    pub removed: bool,
    pub branch_deleted: bool,
    pub branch_kept_reason: Option<String>,
    pub derived_data_removed: Vec<PathBuf>,
    pub watchman_unwatched: bool,
    /// "removed" | "deferred" | "not_needed".
    pub admin_cleanup: String,
}

impl Report for RmReport {
    fn human(&self) -> String {
        let verb = if self.dry_run { "would remove" } else { "removed" };
        let mut s = format!("{verb} {}", self.path.display());
        match (&self.branch, self.branch_deleted) {
            (Some(b), true) => s.push_str(&format!("\n  branch {b}: {}", if self.dry_run { "would delete" } else { "deleted" })),
            (Some(b), false) => s.push_str(&format!("\n  branch {b}: kept ({})", self.branch_kept_reason.as_deref().unwrap_or("-"))),
            _ => {}
        }
        for d in &self.derived_data_removed {
            s.push_str(&format!("\n  derived data: {}", d.display()));
        }
        s
    }
}

pub fn rm(ctx: &Ctx, cwd: &Path, args: &RmArgs) -> Result<RmReport> {
    rm_impl(ctx, cwd, args, &Env::real())
}

fn rm_impl(ctx: &Ctx, cwd: &Path, args: &RmArgs, env: &Env) -> Result<RmReport> {
    let wt = git::resolve_worktree(cwd, &args.name)?;
    let p = wt.path.display().to_string();
    if wt.is_main || wt.bare {
        return Err(CmwError::refused(format!("{p} is the main checkout; cmw never removes it"), "cmw ls"));
    }
    if wt.locked {
        return Err(CmwError::refused(format!("{p} is locked"), format!("git worktree unlock {}", sh_quote(&p))));
    }
    let wts = git::list_worktrees(cwd)?;
    let common = git::common_dir(cwd)?;
    let repo = repo_cwd(&wts, &common);
    let default = git::default_branch(&repo).unwrap_or_else(|_| "main".to_string());
    let opts = StatusOpts { sizes: false, use_gh: env.use_gh };
    let st = inventory::status(ctx, &repo, &wt, &default, &opts, &mut GhCache::default());
    let force_hint = format!("cmw rm {} --force", sh_quote(&args.name));

    if !args.force {
        if st.prunable && st.path.is_dir() {
            return Err(CmwError::refused(
                format!("{p}: worktree link is broken (.git file missing); cmw cannot verify uncommitted work. Repair it, or pass --force to remove it anyway"),
                format!("git worktree repair {}", sh_quote(&p)),
            ));
        }
        if let Some(e) = &st.error {
            return Err(CmwError::refused(format!("could not inspect {p}: {e}"), force_hint));
        }
        if st.dirty > 0 {
            return Err(CmwError::refused(format!("{} uncommitted changes in {p}", st.dirty), force_hint));
        }
        if st.has_submodules {
            return Err(CmwError::refused(
                format!("{p}: worktree has initialized submodules; cmw cannot verify their unpushed commits. Pass --force to remove it anyway"),
                force_hint,
            ));
        }
        if st.unpushed > 0 && st.merged != Some(true) {
            let hint = match &st.branch {
                Some(b) => format!("git -C {} push -u origin {}", sh_quote(&p), sh_quote(b)),
                None => format!("git -C {} switch -c rescue", sh_quote(&p)),
            };
            return Err(CmwError::refused(
                format!("{} unpushed commits in {p} that are not merged into {default}; push them first, or pass --force to discard", st.unpushed),
                hint,
            ));
        }
    }

    let dd = scan_derived_data(&env.dd_root);
    let r = remove_worktree(ctx, &repo, &st, &dd, &env.trash, &RemoveOpts { dry_run: args.dry_run, sync: false, keep_branch: args.keep_branch })?;
    Ok(RmReport {
        path: wt.path,
        branch: wt.branch,
        dry_run: args.dry_run,
        removed: true,
        branch_deleted: r.branch_deleted,
        branch_kept_reason: r.branch_kept_reason,
        derived_data_removed: r.derived_data,
        watchman_unwatched: r.watchman,
        admin_cleanup: r.admin_cleanup.to_string(),
    })
}

// ---------------------------------------------------------------------------------------------
// gc
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct GcItem {
    pub repo: String,
    pub path: PathBuf,
    pub branch: Option<String>,
    pub merged_via: Option<String>,
    pub branch_deleted: bool,
    pub admin_cleanup: String,
}

#[derive(Debug, Serialize)]
pub struct PrunedItem {
    pub repo: String,
    pub path: PathBuf,
    /// "pruned" | "would_prune" (dry run) | "failed".
    pub outcome: String,
}

#[derive(Debug, Serialize)]
pub struct StaleItem {
    pub repo: String,
    pub path: PathBuf,
    pub branch: Option<String>,
    pub idle_days: u64,
}

#[derive(Debug, Serialize)]
pub struct OrphanItem {
    pub repo: String,
    pub path: PathBuf,
    pub hint: String,
}

#[derive(Debug, Serialize)]
pub struct SkippedItem {
    pub repo: String,
    pub path: PathBuf,
    pub branch: Option<String>,
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct GcReport {
    pub dry_run: bool,
    pub removed: Vec<GcItem>,
    pub would_remove: Vec<GcItem>,
    pub pruned: Vec<PrunedItem>,
    pub stale: Vec<StaleItem>,
    pub orphans: Vec<OrphanItem>,
    /// DerivedData folders removed (or, in a dry run, that would be).
    pub derived_data: Vec<PathBuf>,
    pub skipped: Vec<SkippedItem>,
    /// Change in free disk space across the run. Only present with --yes.
    pub freed_bytes: Option<u64>,
    #[serde(skip)]
    all: bool,
}

impl Report for GcReport {
    fn human(&self) -> String {
        let mut s: Vec<String> = Vec::new();
        let (items, verb) = if self.dry_run { (&self.would_remove, "would remove") } else { (&self.removed, "removed") };
        if !items.is_empty() {
            s.push(format!("{verb} {} merged worktree(s):", items.len()));
            for i in items {
                let b = i.branch.as_deref().unwrap_or("(detached)");
                let via = i.merged_via.as_deref().unwrap_or("-");
                let del = if i.branch_deleted { ", branch deleted" } else { "" };
                s.push(format!("  {b}  [{via}{del}]  {}", i.path.display()));
            }
        }
        if !self.pruned.is_empty() {
            s.push(format!("{} {} worktree entr(ies) whose directory is missing:", if self.dry_run { "would prune" } else { "pruned" }, self.pruned.len()));
            s.extend(self.pruned.iter().map(|p| format!("  {}  [{}]", p.path.display(), p.outcome)));
        }
        if !self.derived_data.is_empty() {
            s.push(format!("{} {} DerivedData folder(s):", if self.dry_run { "would delete" } else { "deleted" }, self.derived_data.len()));
            s.extend(self.derived_data.iter().map(|p| format!("  {}", p.display())));
        }
        if !self.stale.is_empty() {
            s.push("stale (not merged; never removed automatically):".to_string());
            s.extend(self.stale.iter().map(|i| format!("  {}  idle {}d  {}", i.branch.as_deref().unwrap_or("(detached)"), i.idle_days, i.path.display())));
        }
        if !self.orphans.is_empty() {
            s.push("orphans (directories not registered as worktrees; never deleted automatically):".to_string());
            s.extend(self.orphans.iter().map(|o| format!("  {}  ->  {}", o.path.display(), o.hint)));
        }
        if !self.skipped.is_empty() {
            s.push("skipped:".to_string());
            s.extend(self.skipped.iter().map(|i| format!("  {}  {}  ({})", i.branch.as_deref().unwrap_or("(detached)"), i.path.display(), i.reason)));
        }
        let actionable = !self.would_remove.is_empty() || !self.pruned.is_empty() || !self.derived_data.is_empty();
        if self.dry_run {
            if actionable {
                s.push(format!("run: cmw gc --yes{}", if self.all { " --all" } else { "" }));
            } else if s.is_empty() {
                s.push("nothing to clean up".to_string());
            }
        } else {
            if s.is_empty() {
                s.push("nothing to clean up".to_string());
            }
            if let Some(b) = self.freed_bytes {
                s.push(format!("freed ~{} (change in free disk space)", human_bytes(b)));
            }
        }
        s.join("\n")
    }
}

fn path_is_inside(cwd: &Path, dir: &Path) -> bool {
    canonical(cwd).starts_with(canonical(dir))
}

/// Directories under `<wt_root>/<repo>/` that git does not know as worktrees.
fn find_orphans(wt_root: &Path, repo: &str, registered: &BTreeSet<PathBuf>) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(wt_root.join(repo)) else { return Vec::new() };
    let mut out: Vec<PathBuf> = rd
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .filter(|p| !registered.contains(&canonical(p)))
        .filter(|p| {
            // A live gitdir means another repo (same name) owns this worktree.
            let gitdir = std::fs::read_to_string(p.join(".git"))
                .ok()
                .and_then(|t| t.trim().strip_prefix("gitdir:").map(|g| PathBuf::from(g.trim())));
            !gitdir.is_some_and(|g| g.is_dir())
        })
        .collect();
    out.sort();
    out
}

pub fn gc(ctx: &Ctx, cwd: &Path, args: &GcArgs) -> Result<GcReport> {
    gc_impl(ctx, cwd, args, &Env::real())
}

fn gc_impl(ctx: &Ctx, cwd: &Path, args: &GcArgs, env: &Env) -> Result<GcReport> {
    let repos = inventory::discover(cwd, args.all, &env.scan_roots)?;
    let dd = scan_derived_data(&env.dd_root);
    let mut report = GcReport {
        dry_run: !args.yes,
        removed: vec![],
        would_remove: vec![],
        pruned: vec![],
        stale: vec![],
        orphans: vec![],
        derived_data: vec![],
        skipped: vec![],
        freed_bytes: None,
        all: args.all,
    };

    let probe = args.yes.then(|| {
        let mut probes: Vec<&Path> = vec![env.wt_root.as_path(), env.dd_root.as_path()];
        probes.extend(env.scan_roots.iter().map(PathBuf::as_path));
        probes.extend(repos.iter().map(|r| r.common_dir.as_path()));
        FreeProbe::new(probes)
    });

    let mut orphan_seen: BTreeSet<PathBuf> = BTreeSet::new();
    for repo in &repos {
        gc_repo(ctx, repo, args, env, &dd, cwd, &mut report, &mut orphan_seen);
    }

    // DerivedData of workspaces that no longer exist.
    for d in dd_orphans(&dd) {
        if args.yes {
            match delete_path(&d) {
                Ok(()) => report.derived_data.push(d),
                Err(e) => ctx.warn(format!("could not delete DerivedData {}: {e}", d.display())),
            }
        } else {
            report.derived_data.push(d);
        }
    }
    report.freed_bytes = probe.map(|p| p.freed());
    Ok(report)
}

#[allow(clippy::too_many_arguments)]
fn gc_repo(ctx: &Ctx, repo: &RepoRef, args: &GcArgs, env: &Env, dd: &[DdEntry], cwd: &Path, report: &mut GcReport, orphan_seen: &mut BTreeSet<PathBuf>) {
    // Only `--all --yes` is unlocked by main; the current repo is already locked there.
    let _guard = if args.all && args.yes {
        match lock::acquire(&repo.common_dir, LOCK_TIMEOUT) {
            Ok(g) => Some(g),
            Err(e) => {
                ctx.warn(format!("skipping {}: {}", repo.common_dir.display(), e.message));
                return;
            }
        }
    } else {
        None
    };
    let wts = match git::list_worktrees(&repo.cwd) {
        Ok(w) => w,
        Err(e) => {
            ctx.warn(format!("{}: {}", repo.common_dir.display(), e.message));
            return;
        }
    };
    let name = git::repo_name(&repo.common_dir);
    let repo_dir = repo_cwd(&wts, &repo.common_dir);
    let default = git::default_branch(&repo_dir).unwrap_or_else(|_| "main".to_string());
    let opts = StatusOpts { sizes: false, use_gh: env.use_gh };
    let mut gh = GhCache::default();
    let mut candidates: Vec<WorktreeStatus> = Vec::new();

    for wt in wts.iter().filter(|w| !w.is_main && !w.bare) {
        let skip = |reason: String| SkippedItem { repo: name.clone(), path: wt.path.clone(), branch: wt.branch.clone(), reason };
        if wt.prunable && wt.path.is_dir() {
            // Directory exists: git cannot use it, but it may hold real work. Never prune or remove.
            report.skipped.push(skip(format!("worktree link is broken; cmw cannot verify uncommitted work (git worktree repair {})", wt.path.display())));
            continue;
        }
        if wt.prunable {
            // Directory is gone: drop exactly this entry's admin dir, only once it is proven to be ours.
            // Never a repo-wide `git worktree prune`, which would also destroy broken-link registrations.
            match resolve_admin(&repo_dir, &wt.path) {
                _ if wt.locked => report.skipped.push(skip("locked".into())),
                None => report.skipped.push(skip("cannot prove which git registration belongs to this missing worktree; left untouched".into())),
                Some(admin) => {
                    let outcome = if !args.yes {
                        "would_prune"
                    } else if std::fs::remove_dir_all(&admin).is_ok() {
                        "pruned"
                    } else {
                        ctx.warn(format!("could not remove {}", admin.display()));
                        "failed"
                    };
                    report.pruned.push(PrunedItem { repo: name.clone(), path: wt.path.clone(), outcome: outcome.into() });
                }
            }
            continue;
        }
        let st = inventory::status(ctx, &repo_dir, wt, &default, &opts, &mut gh);
        let merged = st.merged == Some(true);
        if merged && !st.locked && st.dirty == 0 && st.error.is_none() && !st.has_submodules {
            if path_is_inside(cwd, &wt.path) {
                report.skipped.push(skip("the current directory is inside this worktree".into()));
            } else {
                candidates.push(st);
            }
        } else if let Some(days) = args.stale.filter(|d| st.idle_days.is_some_and(|i| i >= *d)).and(st.idle_days) {
            report.stale.push(StaleItem { repo: name.clone(), path: wt.path.clone(), branch: wt.branch.clone(), idle_days: days });
        } else if merged {
            let reason = if st.locked {
                "locked".to_string()
            } else if let Some(e) = &st.error {
                format!("could not inspect: {e}")
            } else if st.has_submodules {
                "worktree has initialized submodules; cmw cannot verify their unpushed commits".to_string()
            } else {
                format!("{} uncommitted changes", st.dirty)
            };
            report.skipped.push(skip(reason));
        }
    }

    for st in candidates {
        let item = |branch_deleted: bool, admin: &str| GcItem {
            admin_cleanup: admin.to_string(),
            repo: name.clone(),
            path: st.path.clone(),
            branch: st.branch.clone(),
            merged_via: st.merged_via.clone(),
            branch_deleted,
        };
        let o = RemoveOpts { dry_run: !args.yes, sync: true, keep_branch: false };
        match remove_worktree(ctx, &repo_dir, &st, dd, &env.trash, &o) {
            Ok(r) => {
                report.derived_data.extend(r.derived_data);
                if args.yes {
                    report.removed.push(item(r.branch_deleted, r.admin_cleanup));
                } else {
                    report.would_remove.push(item(r.branch_deleted, r.admin_cleanup));
                }
            }
            Err(e) => {
                ctx.warn(format!("could not remove {}: {}", st.path.display(), e.message));
                report.skipped.push(SkippedItem { repo: name.clone(), path: st.path.clone(), branch: st.branch.clone(), reason: format!("removal failed: {}", e.message) });
            }
        }
    }

    let registered: BTreeSet<PathBuf> = wts.iter().map(|w| canonical(&w.path)).collect();
    for o in find_orphans(&env.wt_root, &name, &registered) {
        if orphan_seen.insert(o.clone()) {
            let hint = format!("rm -rf {}", sh_quote(&o.display().to_string()));
            report.orphans.push(OrphanItem { repo: name.clone(), path: o, hint });
        }
    }
}

// ---------------------------------------------------------------------------------------------
// clean-artifacts
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ArtifactItem {
    pub path: PathBuf,
    pub apparent_bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct ArtifactWorktree {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub items: Vec<ArtifactItem>,
}

#[derive(Debug, Serialize)]
pub struct CleanArtifactsReport {
    pub dry_run: bool,
    pub worktrees: Vec<ArtifactWorktree>,
    /// Apparent size of everything listed; APFS clones are counted at full size.
    pub total_apparent_bytes: u64,
    /// Change in free disk space across the run. Only present with --yes.
    pub freed_bytes: Option<u64>,
}

impl Report for CleanArtifactsReport {
    fn human(&self) -> String {
        if self.worktrees.is_empty() {
            return "no idle worktrees with reclaimable artifacts".into();
        }
        let mut s: Vec<String> = Vec::new();
        for w in &self.worktrees {
            s.push(format!("{}  {}", w.branch.as_deref().unwrap_or("(detached)"), w.path.display()));
            s.extend(w.items.iter().map(|i| format!("  {:>10}  {}", human_bytes(i.apparent_bytes), i.path.display())));
        }
        match self.freed_bytes {
            Some(b) => s.push(format!("deleted; freed ~{} (change in free disk space)", human_bytes(b))),
            None => {
                s.push(format!("{} apparent (APFS clones counted at full size; real savings are lower)", human_bytes(self.total_apparent_bytes)));
                s.push("run: cmw clean-artifacts --yes".to_string());
            }
        }
        s.join("\n")
    }
}

fn has_tracked(wt: &Path, rel: &str) -> bool {
    git::run_raw(wt, &["ls-files", "-z", "--", rel.trim_end_matches('/')]).map(|o| !o.is_empty()).unwrap_or(true)
}

/// Reclaimable directories nested inside an ignored generated native project (`ios/`, `android/`).
fn nested_reclaimable(wt: &Path, top: &str, rules: &rules::RuleSet, out: &mut Vec<String>) {
    let mut stack = vec![(top.trim_end_matches('/').to_string(), 0u8)];
    while let Some((rel, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(wt.join(&rel)) else { continue };
        for e in rd.flatten() {
            if !e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_string();
            let child = format!("{rel}/{name}");
            if rules.classify(&child, true).is_some_and(|r| r.reclaimable) {
                out.push(format!("{child}/"));
            } else if depth < 4 && name != "node_modules" && name != "Pods" {
                stack.push((child, depth + 1));
            }
        }
    }
}

/// Git-ignored, reclaimable paths of one worktree (absolute).
fn artifact_paths(wt: &Path, rules: &rules::RuleSet) -> Result<Vec<PathBuf>> {
    let mut rels: Vec<String> = Vec::new();
    for entry in git::ignored_entries(wt)? {
        let is_dir = entry.ends_with('/');
        match rules.classify(&entry, is_dir) {
            Some(r) if r.reclaimable => rels.push(entry),
            Some(r) if r.native_project && is_dir => nested_reclaimable(wt, &entry, rules, &mut rels),
            _ => {}
        }
    }
    rels.sort();
    rels.dedup();
    Ok(rels
        .into_iter()
        // Defense in depth: never delete anything git tracks.
        .filter(|r| !r.split('/').any(|c| c == "..") && !has_tracked(wt, r))
        .map(|r| wt.join(r.trim_end_matches('/')))
        .collect())
}

pub fn clean_artifacts(ctx: &Ctx, cwd: &Path, args: &CleanArtifactsArgs) -> Result<CleanArtifactsReport> {
    clean_artifacts_impl(ctx, cwd, args, &Env::real())
}

fn clean_artifacts_impl(ctx: &Ctx, cwd: &Path, args: &CleanArtifactsArgs, env: &Env) -> Result<CleanArtifactsReport> {
    let repos = inventory::discover(cwd, args.all, &env.scan_roots)?;
    let dd = scan_derived_data(&env.dd_root);
    let probe = args.yes.then(|| {
        let mut probes: Vec<&Path> = vec![env.dd_root.as_path()];
        probes.extend(env.scan_roots.iter().map(PathBuf::as_path));
        probes.extend(repos.iter().map(|r| r.common_dir.as_path()));
        FreeProbe::new(probes)
    });
    let mut worktrees = Vec::new();

    for repo in &repos {
        let _guard = if args.all && args.yes {
            match lock::acquire(&repo.common_dir, LOCK_TIMEOUT) {
                Ok(g) => Some(g),
                Err(e) => {
                    ctx.warn(format!("skipping {}: {}", repo.common_dir.display(), e.message));
                    continue;
                }
            }
        } else {
            None
        };
        let wts = match git::list_worktrees(&repo.cwd) {
            Ok(w) => w,
            Err(e) => {
                ctx.warn(format!("{}: {}", repo.common_dir.display(), e.message));
                continue;
            }
        };
        for wt in wts.iter().filter(|w| !w.is_main && !w.bare && !w.prunable && w.path.is_dir()) {
            let (_, idle) = inventory::activity(&wt.path);
            if !idle.is_some_and(|d| d >= args.idle) {
                continue;
            }
            let rules = match config::ruleset(&wt.path) {
                Ok((r, _)) => r,
                Err(e) => {
                    ctx.warn(format!("{}: {}", wt.path.display(), e.message));
                    continue;
                }
            };
            let mut targets = match artifact_paths(&wt.path, &rules) {
                Ok(t) => t,
                Err(e) => {
                    ctx.warn(format!("{}: {}", wt.path.display(), e.message));
                    continue;
                }
            };
            targets.extend(dd_for_worktree(&dd, &wt.path));
            if targets.is_empty() {
                continue;
            }
            let items: Vec<ArtifactItem> = targets.into_iter().map(|p| ArtifactItem { apparent_bytes: inventory::apparent_size(&p), path: p }).collect();
            if args.yes {
                for i in &items {
                    if let Err(e) = delete_path(&i.path) {
                        ctx.warn(format!("could not delete {}: {e}", i.path.display()));
                    }
                }
            }
            worktrees.push(ArtifactWorktree { path: wt.path.clone(), branch: wt.branch.clone(), items });
        }
    }
    let total = worktrees.iter().flat_map(|w| &w.items).map(|i| i.apparent_bytes).sum();
    Ok(CleanArtifactsReport { dry_run: !args.yes, worktrees, total_apparent_bytes: total, freed_bytes: probe.map(|p| p.freed()) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::testutil::*;
    use crate::output::ErrorKind;

    fn env(tmp: &Path) -> Env {
        Env {
            use_gh: false,
            dd_root: tmp.join("dd"),
            trash: tmp.join("trash"),
            wt_root: tmp.join("cmw-wts"),
            scan_roots: vec![],
        }
    }

    fn ctx() -> Ctx {
        Ctx::new(true, "test")
    }

    fn rm_args(name: &str) -> RmArgs {
        RmArgs { name: name.into(), force: false, keep_branch: false, dry_run: false }
    }

    fn gc_args(yes: bool) -> GcArgs {
        GcArgs { yes, stale: None, all: false }
    }

    fn branch_exists(repo: &Path, b: &str) -> bool {
        git::run_ok(repo, &["show-ref", "--verify", "--quiet", &format!("refs/heads/{b}")])
    }

    #[test]
    fn dirty_worktree_is_refused_with_hint() {
        let (t, repo) = repo();
        let wt = add_worktree(&repo, "dirty");
        std::fs::write(wt.join("wip.txt"), "wip").unwrap();
        let e = rm_impl(&ctx(), &repo, &rm_args("dirty"), &env(t.path())).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Refused);
        assert!(e.message.contains("1 uncommitted changes"), "{}", e.message);
        assert_eq!(e.hint.as_deref(), Some("cmw rm dirty --force"));
        assert!(wt.exists());
        // --force removes it, but keeps the unmerged-or-unknown branch.
        let mut a = rm_args("dirty");
        a.force = true;
        let r = rm_impl(&ctx(), &repo, &a, &env(t.path())).unwrap();
        assert!(r.removed && !wt.exists());
        assert!(branch_exists(&repo, "dirty"));
        assert!(git::list_worktrees(&repo).unwrap().len() == 1);
    }

    #[test]
    fn unpushed_unmerged_is_refused() {
        let (t, repo) = repo();
        let wt = add_worktree(&repo, "work");
        commit_file(&wt, "a.txt");
        let e = rm_impl(&ctx(), &repo, &rm_args("work"), &env(t.path())).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Refused);
        assert!(e.message.contains("1 unpushed") && e.message.contains("--force"), "{}", e.message);
        assert!(e.hint.unwrap().contains("push -u origin work"));
        assert!(wt.exists());
    }

    #[test]
    fn main_and_locked_are_refused() {
        let (t, repo) = repo();
        let e = rm_impl(&ctx(), &repo, &rm_args(repo.to_str().unwrap()), &env(t.path())).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Refused);
        let mut forced = rm_args("main");
        forced.force = true;
        assert_eq!(rm_impl(&ctx(), &repo, &forced, &env(t.path())).unwrap_err().kind, ErrorKind::Refused);
        assert!(repo.join("README.md").exists());

        let wt = add_worktree(&repo, "lk");
        git(&repo, &["worktree", "lock", wt.to_str().unwrap()]);
        let e = rm_impl(&ctx(), &repo, &forced_for("lk"), &env(t.path())).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Refused);
        assert!(e.hint.unwrap().starts_with("git worktree unlock"));
    }

    fn forced_for(name: &str) -> RmArgs {
        let mut a = rm_args(name);
        a.force = true;
        a
    }

    #[test]
    fn fresh_branch_is_not_a_gc_candidate() {
        let (t, repo) = repo();
        let wt = add_worktree(&repo, "fresh");
        let r = gc_impl(&ctx(), &repo, &gc_args(true), &env(t.path())).unwrap();
        assert!(r.removed.is_empty() && r.would_remove.is_empty());
        assert!(wt.exists() && branch_exists(&repo, "fresh"));
    }

    #[test]
    fn merged_branch_is_removed_by_gc_yes() {
        let (t, repo) = repo();
        let wt = add_worktree(&repo, "done");
        commit_file(&wt, "a.txt");
        git(&repo, &["merge", "--no-edit", "done"]);
        let other = add_worktree(&repo, "other");
        commit_file(&other, "b.txt");

        let dry = gc_impl(&ctx(), &repo, &gc_args(false), &env(t.path())).unwrap();
        assert_eq!(dry.would_remove.len(), 1);
        assert!(dry.freed_bytes.is_none());
        assert!(dry.human().ends_with("run: cmw gc --yes"), "{}", dry.human());
        assert!(wt.exists());

        let r = gc_impl(&ctx(), &repo, &gc_args(true), &env(t.path())).unwrap();
        assert_eq!(r.removed.len(), 1);
        assert!(r.removed[0].branch_deleted && r.freed_bytes.is_some());
        assert!(!wt.exists());
        assert!(!branch_exists(&repo, "done"));
        assert!(other.exists() && branch_exists(&repo, "other"));
        assert_eq!(git::list_worktrees(&repo).unwrap().len(), 2);
    }

    #[test]
    fn dirty_merged_worktree_is_skipped_and_prunable_is_pruned() {
        let (t, repo) = repo();
        let wt = add_worktree(&repo, "dm");
        commit_file(&wt, "a.txt");
        git(&repo, &["merge", "--no-edit", "dm"]);
        std::fs::write(wt.join("wip"), "x").unwrap();
        let gone = add_worktree(&repo, "gone");
        std::fs::remove_dir_all(&gone).unwrap();
        let r = gc_impl(&ctx(), &repo, &gc_args(true), &env(t.path())).unwrap();
        assert!(r.removed.is_empty());
        assert_eq!(r.skipped.len(), 1);
        assert_eq!(r.pruned.len(), 1);
        assert!(wt.exists());
        assert_eq!(git::list_worktrees(&repo).unwrap().len(), 2);
    }

    fn break_link(wt: &Path) {
        std::fs::write(wt.join("precious"), "x").unwrap();
        std::fs::remove_file(wt.join(".git")).unwrap();
    }

    #[test]
    fn rm_refuses_existing_worktree_with_broken_link() {
        let (t, repo) = repo();
        let wt = add_worktree(&repo, "broken");
        break_link(&wt);
        let e = rm_impl(&ctx(), &repo, &rm_args("broken"), &env(t.path())).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Refused);
        assert!(e.message.contains("--force"), "{}", e.message);
        assert!(e.hint.unwrap().starts_with("git worktree repair"));
        assert!(wt.join("precious").exists());
    }

    #[test]
    fn gc_never_removes_or_prunes_existing_broken_worktree() {
        let (t, repo) = repo();
        let wt = add_worktree(&repo, "broken");
        break_link(&wt);
        let r = gc_impl(&ctx(), &repo, &gc_args(true), &env(t.path())).unwrap();
        assert!(r.removed.is_empty() && r.pruned.is_empty());
        assert_eq!(r.skipped.len(), 1);
        assert!(wt.join("precious").exists());
        assert_eq!(git::list_worktrees(&repo).unwrap().len(), 2);
    }

    #[test]
    fn gc_prunes_only_missing_dirs_and_keeps_broken_registration() {
        let (t, repo) = repo();
        let broken = add_worktree(&repo, "broken");
        break_link(&broken);
        let gone = add_worktree(&repo, "gone");
        std::fs::remove_dir_all(&gone).unwrap();
        let common = canonical(&git::common_dir(&repo).unwrap());
        let admin_of = |n: &str| common.join("worktrees").join(n);
        let r = gc_impl(&ctx(), &repo, &gc_args(true), &env(t.path())).unwrap();
        assert_eq!(r.pruned.len(), 1);
        assert_eq!(r.pruned[0].outcome, "pruned");
        assert_eq!(r.skipped.len(), 1);
        assert!(admin_of("broken").exists(), "broken admin dir was destroyed");
        assert!(!admin_of("gone").exists());
        assert!(broken.join("precious").exists() && !gone.exists());
        let paths: Vec<_> = git::list_worktrees(&repo).unwrap().into_iter().map(|w| canonical(&w.path)).collect();
        assert!(paths.contains(&canonical(&broken)));
        assert!(!paths.contains(&canonical(&gone)));
    }

    #[test]
    fn gc_dry_run_reports_prune_without_deleting() {
        let (t, repo) = repo();
        let gone = add_worktree(&repo, "gone");
        std::fs::remove_dir_all(&gone).unwrap();
        let r = gc_impl(&ctx(), &repo, &gc_args(false), &env(t.path())).unwrap();
        assert_eq!(r.pruned.len(), 1);
        assert_eq!(r.pruned[0].outcome, "would_prune");
        assert_eq!(git::list_worktrees(&repo).unwrap().len(), 2);
    }

    #[test]
    fn rm_never_prunes_unrelated_entries_when_admin_is_unresolved() {
        let (t, repo) = repo();
        let unmounted = add_worktree(&repo, "unmounted");
        std::fs::remove_dir_all(&unmounted).unwrap();
        let wt = add_worktree(&repo, "x");
        break_link(&wt);
        let r = rm_impl(&ctx(), &repo, &forced_for("x"), &env(t.path())).unwrap();
        assert_eq!(r.admin_cleanup, "deferred");
        assert!(!wt.exists());
        let paths: Vec<_> = git::list_worktrees(&repo).unwrap().into_iter().map(|w| w.path).collect();
        assert!(paths.contains(&unmounted), "unrelated registration was pruned");
    }

    #[test]
    fn rm_removes_exactly_its_admin_dir() {
        let (t, repo) = repo();
        let other = add_worktree(&repo, "keep");
        let wt = add_worktree(&repo, "gone");
        let r = rm_impl(&ctx(), &repo, &rm_args("gone"), &env(t.path())).unwrap();
        assert_eq!(r.admin_cleanup, "removed");
        let wts = git::list_worktrees(&repo).unwrap();
        assert_eq!(wts.len(), 2);
        assert!(other.exists() && !wt.exists());
    }

    fn submodule_worktree(t: &Path, repo: &Path, name: &str) -> PathBuf {
        let sub = t.join("subsrc");
        std::fs::create_dir(&sub).unwrap();
        git(&sub, &["init", "-q", "-b", "main"]);
        git(&sub, &["config", "user.email", "t@example.com"]);
        git(&sub, &["config", "user.name", "T"]);
        commit_file(&sub, "f");
        git(repo, &["-c", "protocol.file.allow=always", "submodule", "add", "-q", sub.to_str().unwrap(), "libs/sub"]);
        git(repo, &["commit", "-q", "-m", "add sub"]);
        let wt = add_worktree(repo, name);
        git(&wt, &["-c", "protocol.file.allow=always", "submodule", "update", "--init", "-q"]);
        wt
    }

    #[test]
    fn rm_refuses_worktree_with_initialized_submodules() {
        let (t, repo) = repo();
        let wt = submodule_worktree(t.path(), &repo, "smod");
        let e = rm_impl(&ctx(), &repo, &rm_args("smod"), &env(t.path())).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Refused);
        assert!(e.message.contains("initialized submodules"), "{}", e.message);
        assert_eq!(e.hint.as_deref(), Some("cmw rm smod --force"));
        assert!(wt.exists());
    }

    #[test]
    fn gc_skips_merged_worktree_with_initialized_submodules() {
        let (t, repo) = repo();
        let wt = submodule_worktree(t.path(), &repo, "smod");
        commit_file(&wt, "a.txt");
        git(&repo, &["merge", "--no-edit", "smod"]);
        let r = gc_impl(&ctx(), &repo, &gc_args(true), &env(t.path())).unwrap();
        assert!(r.removed.is_empty());
        assert!(r.skipped.iter().any(|s| s.reason.contains("initialized submodules")), "{:?}", r.skipped);
        assert!(wt.exists());
    }

    #[test]
    fn stale_is_reported_but_never_removed() {
        let (t, repo) = repo();
        let wt = add_worktree(&repo, "old");
        commit_file(&wt, "a.txt");
        let mut a = gc_args(true);
        a.stale = Some(0);
        let r = gc_impl(&ctx(), &repo, &a, &env(t.path())).unwrap();
        assert_eq!(r.stale.len(), 1);
        assert!(wt.exists());
    }

    #[test]
    fn orphan_dirs_are_reported_not_deleted() {
        let (t, repo) = repo();
        let e = env(t.path());
        let orphan = e.wt_root.join("repo").join("leftover");
        std::fs::create_dir_all(&orphan).unwrap();
        let r = gc_impl(&ctx(), &repo, &gc_args(true), &e).unwrap();
        assert_eq!(r.orphans.len(), 1);
        assert!(r.orphans[0].hint.starts_with("rm -rf "));
        assert!(orphan.exists());
    }

    fn write_plist(dd: &Path, name: &str, workspace: &Path) -> PathBuf {
        let dir = dd.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("info.plist"),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n\t<key>WorkspacePath</key>\n\t<string>{}</string>\n</dict>\n</plist>\n",
                workspace.display()
            ),
        )
        .unwrap();
        dir
    }

    #[test]
    fn rm_removes_matching_derived_data_only() {
        let (t, repo) = repo();
        let e = env(t.path());
        let wt = add_worktree(&repo, "ios");
        let mine = write_plist(&e.dd_root, "App-aaa", &wt.join("ios/App.xcworkspace"));
        let theirs = write_plist(&e.dd_root, "App-bbb", &repo.join("ios/App.xcworkspace"));
        let r = rm_impl(&ctx(), &repo, &rm_args("ios"), &e).unwrap();
        assert_eq!(r.derived_data_removed, vec![mine.clone()]);
        assert!(!mine.exists() && theirs.exists() && !wt.exists());
    }

    #[test]
    fn gc_yes_removes_orphaned_derived_data() {
        let (t, repo) = repo();
        let e = env(t.path());
        let orphan = write_plist(&e.dd_root, "Gone-ccc", &t.path().join("deleted/App.xcworkspace"));
        std::fs::create_dir_all(repo.join("App.xcworkspace")).unwrap();
        let live = write_plist(&e.dd_root, "Live-ddd", &repo.join("App.xcworkspace"));
        let dry = gc_impl(&ctx(), &repo, &gc_args(false), &e).unwrap();
        assert_eq!(dry.derived_data, vec![orphan.clone()]);
        assert!(orphan.exists());
        let r = gc_impl(&ctx(), &repo, &gc_args(true), &e).unwrap();
        assert_eq!(r.derived_data, vec![orphan.clone()]);
        assert!(!orphan.exists() && live.exists());
    }

    #[test]
    fn rm_dry_run_changes_nothing() {
        let (t, repo) = repo();
        let wt = add_worktree(&repo, "dry");
        let mut a = rm_args("dry");
        a.dry_run = true;
        let r = rm_impl(&ctx(), &repo, &a, &env(t.path())).unwrap();
        assert!(r.dry_run && wt.exists() && branch_exists(&repo, "dry"));
    }

    #[test]
    fn rm_deletes_merged_branch_unless_kept() {
        let (t, repo) = repo();
        let wt = add_worktree(&repo, "m1");
        commit_file(&wt, "a.txt");
        git(&repo, &["merge", "--no-edit", "m1"]);
        let r = rm_impl(&ctx(), &repo, &rm_args("m1"), &env(t.path())).unwrap();
        assert!(r.branch_deleted && !branch_exists(&repo, "m1"));

        let wt = add_worktree(&repo, "m2");
        commit_file(&wt, "b.txt");
        git(&repo, &["merge", "--no-edit", "m2"]);
        let mut a = rm_args("m2");
        a.keep_branch = true;
        let r = rm_impl(&ctx(), &repo, &a, &env(t.path())).unwrap();
        assert!(!r.branch_deleted && branch_exists(&repo, "m2"));
        assert_eq!(r.branch_kept_reason.as_deref(), Some("--keep-branch"));
    }

    #[test]
    fn clean_artifacts_deletes_ignored_build_keeps_tracked() {
        let (t, repo) = repo();
        let wt = add_worktree(&repo, "art");
        std::fs::create_dir_all(wt.join("build")).unwrap();
        std::fs::write(wt.join("build/out.bin"), vec![0u8; 4096]).unwrap();
        let args = CleanArtifactsArgs { idle: 0, yes: false, all: false };
        let dry = clean_artifacts_impl(&ctx(), &repo, &args, &env(t.path())).unwrap();
        assert_eq!(dry.worktrees.len(), 1);
        assert_eq!(dry.worktrees[0].items.len(), 1);
        assert!(dry.worktrees[0].items[0].apparent_bytes >= 4096);
        assert!(wt.join("build").exists() && dry.freed_bytes.is_none());

        let args = CleanArtifactsArgs { idle: 0, yes: true, all: false };
        let r = clean_artifacts_impl(&ctx(), &repo, &args, &env(t.path())).unwrap();
        assert!(r.freed_bytes.is_some());
        assert!(!wt.join("build").exists());
        assert!(wt.join("README.md").exists() && wt.join(".gitignore").exists());
        assert!(repo.join("README.md").exists());
    }

    #[test]
    fn clean_artifacts_never_deletes_tracked_files_in_reclaimable_dirs() {
        let (t, repo) = repo();
        let wt = add_worktree(&repo, "trk");
        std::fs::create_dir_all(wt.join("build")).unwrap();
        std::fs::write(wt.join("build/keep.txt"), "tracked").unwrap();
        git(&wt, &["add", "-f", "build/keep.txt"]);
        git(&wt, &["commit", "-m", "track build file"]);
        std::fs::write(wt.join("build/junk.bin"), "junk").unwrap();
        let args = CleanArtifactsArgs { idle: 0, yes: true, all: false };
        clean_artifacts_impl(&ctx(), &repo, &args, &env(t.path())).unwrap();
        assert!(wt.join("build/keep.txt").exists());
    }

    #[test]
    fn clean_artifacts_finds_nested_dirs_in_native_projects() {
        let (t, repo) = repo();
        let wt = add_worktree(&repo, "nat");
        std::fs::write(wt.join(".gitignore"), "ios/\n").unwrap();
        git(&wt, &["commit", "-am", "ignore ios"]);
        std::fs::create_dir_all(wt.join("ios/build")).unwrap();
        std::fs::create_dir_all(wt.join("ios/Pods/x/build")).unwrap();
        std::fs::write(wt.join("ios/build/a"), "a").unwrap();
        std::fs::write(wt.join("ios/Pods/x/build/b"), "b").unwrap();
        let args = CleanArtifactsArgs { idle: 0, yes: true, all: false };
        clean_artifacts_impl(&ctx(), &repo, &args, &env(t.path())).unwrap();
        assert!(!wt.join("ios/build").exists());
        assert!(wt.join("ios/Pods/x/build/b").exists(), "must not enter Pods");
    }

    #[test]
    fn clean_artifacts_respects_idle_threshold() {
        let (t, repo) = repo();
        let wt = add_worktree(&repo, "busy");
        std::fs::create_dir_all(wt.join("build")).unwrap();
        std::fs::write(wt.join("build/x"), "x").unwrap();
        let args = CleanArtifactsArgs { idle: 7, yes: true, all: false };
        let r = clean_artifacts_impl(&ctx(), &repo, &args, &env(t.path())).unwrap();
        assert!(r.worktrees.is_empty());
        assert!(wt.join("build/x").exists());
    }

    #[test]
    fn xml_entities_are_decoded() {
        assert_eq!(xml_unescape("a&amp;b&lt;c"), "a&b<c");
    }
}
