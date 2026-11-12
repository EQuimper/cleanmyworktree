//! `cmw ls` / `cmw path` and per-worktree status (shared with `gc`).
use crate::cli::{LsArgs, PathArgs};
use crate::git::{self, Worktree};
use crate::output::{Ctx, Report, Result, human_bytes};
use crate::paths;
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize)]
pub struct WorktreeStatus {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub is_main: bool,
    pub bare: bool,
    pub detached: bool,
    pub locked: bool,
    pub prunable: bool,
    /// Changed or untracked (non-ignored) paths. 0 means clean.
    pub dirty: usize,
    /// The worktree has an initialized submodule; cmw does not inspect their unpushed commits.
    pub has_submodules: bool,
    pub upstream: Option<String>,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    /// Commits reachable from HEAD that no remote-tracking branch has (or, with no remotes, that the default branch lacks).
    pub unpushed: u32,
    /// None when it does not apply (default branch, detached HEAD, main checkout) or is unknown.
    pub merged: Option<bool>,
    /// "gh" | "ancestor" | "patch_id" when merged.
    pub merged_via: Option<String>,
    pub merge_reason: String,
    pub last_commit_unix: Option<i64>,
    pub idle_days: Option<u64>,
    /// cmw hydrate state exists for this worktree.
    pub hydrated: bool,
    pub apparent_bytes: Option<u64>,
    /// Set when git could not be inspected; callers must treat the worktree as unsafe to remove.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct StatusOpts {
    pub sizes: bool,
    pub use_gh: bool,
}

/// Merged-PR list fetched at most once per repo. Create one per repo.
#[derive(Default)]
pub struct GhCache {
    /// None: not fetched yet. Some(None): unavailable or failed. Some(Some(..)): (headRefName, headRefOid).
    prs: Option<Option<Vec<(String, String)>>>,
}

impl GhCache {
    fn has_merged_pr(&mut self, ctx: &Ctx, repo_cwd: &Path, branch: &str, head: &str) -> bool {
        if self.prs.is_none() {
            self.prs = Some(fetch_merged_prs(ctx, repo_cwd));
        }
        self.prs
            .as_ref()
            .and_then(|p| p.as_ref())
            .is_some_and(|list| list.iter().any(|(b, oid)| b == branch && oid == head))
    }
}

fn fetch_merged_prs(ctx: &Ctx, repo_cwd: &Path) -> Option<Vec<(String, String)>> {
    paths::find_in_path("gh")?;
    let url = git::run(repo_cwd, &["remote", "get-url", "origin"]).ok()?;
    if !url.contains("github.com") {
        return None;
    }
    let mut cmd = Command::new("gh");
    cmd.args(["pr", "list", "--state", "merged", "--limit", "200", "--json", "headRefName,headRefOid"])
        .current_dir(repo_cwd)
        .env("GH_PROMPT_DISABLED", "1");
    let parsed = run_capture(cmd, Duration::from_secs(10)).and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
    let Some(serde_json::Value::Array(items)) = parsed else {
        ctx.warn("could not list merged PRs with gh; falling back to git-only merge detection");
        return None;
    };
    Some(
        items
            .iter()
            .filter_map(|i| Some((i.get("headRefName")?.as_str()?.to_string(), i.get("headRefOid")?.as_str()?.to_string())))
            .collect(),
    )
}

/// Run a command with stdin/stderr null, capture stdout, kill it after `timeout`. None on failure or timeout.
fn run_capture(mut cmd: Command, timeout: Duration) -> Option<Vec<u8>> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let out = reader.join().ok()?;
    status.filter(|s| s.success()).map(|_| out)
}

fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// (last commit unix time, idle days). Idle = days since max(last commit, mtime of `<git_dir>/index`).
pub fn activity(wt_path: &Path) -> (Option<i64>, Option<u64>) {
    let last_commit = git::run(wt_path, &["log", "-1", "--format=%ct"]).ok().and_then(|s| s.trim().parse::<i64>().ok());
    let index_mtime = git::git_dir(wt_path)
        .ok()
        .and_then(|gd| std::fs::metadata(gd.join("index")).ok())
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);
    let newest = last_commit.into_iter().chain(index_mtime).max();
    let idle = newest.map(|t| (now_unix() - t).max(0) as u64 / 86_400);
    (last_commit, idle)
}

/// Apparent size: sum of file lengths, symlinks not followed. APFS clones are counted at full size.
pub fn apparent_size(path: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(p) = stack.pop() {
        let Ok(md) = std::fs::symlink_metadata(&p) else { continue };
        if md.is_dir() {
            if let Ok(rd) = std::fs::read_dir(&p) {
                stack.extend(rd.flatten().map(|e| e.path()));
            }
        } else {
            total += md.len();
        }
    }
    total
}

fn ref_exists(cwd: &Path, r: &str) -> bool {
    git::run_ok(cwd, &["show-ref", "--verify", "--quiet", &format!("refs/{r}")])
}

/// (merged, via, reason). Conservative: only says Some(true) with positive evidence.
fn detect_merge(
    ctx: &Ctx,
    repo_cwd: &Path,
    wt: &Worktree,
    default_branch: &str,
    opts: &StatusOpts,
    gh: &mut GhCache,
) -> (Option<bool>, Option<&'static str>, String) {
    let Some(branch) = wt.branch.as_deref() else {
        return (None, None, "detached HEAD".into());
    };
    if wt.is_main {
        return (None, None, "main checkout".into());
    }
    if branch == default_branch {
        return (None, None, "default branch".into());
    }
    let refs: Vec<String> = [format!("origin/{default_branch}"), default_branch.to_string()]
        .into_iter()
        .filter(|r| ref_exists(&wt.path, &if r.starts_with("origin/") { format!("remotes/{r}") } else { format!("heads/{r}") }))
        .collect();
    if let Some(r) = refs.iter().find(|r| git::run_ok(&wt.path, &["merge-base", "--is-ancestor", "HEAD", r])) {
        // A branch with a single reflog entry was just created: its HEAD is trivially an ancestor.
        let entries = git::run(&wt.path, &["reflog", "show", "--format=%H", &format!("refs/heads/{branch}")])
            .map(|s| s.lines().filter(|l| !l.is_empty()).count())
            .unwrap_or(0);
        return if entries > 1 {
            (Some(true), Some("ancestor"), format!("HEAD is an ancestor of {r}"))
        } else {
            (Some(false), None, "no_commits".into())
        };
    }
    // `git cherry` skips merge commits, so patch-id proof only holds for a branch without any.
    let mut cherry_reason = None;
    if let Some(r) = refs.first() {
        match git::run(&wt.path, &["rev-list", "--merges", &format!("{r}..HEAD")]) {
            Ok(m) if m.trim().is_empty() => {
                if let Ok(out) = git::run(&wt.path, &["cherry", r, "HEAD"]) {
                    let lines: Vec<&str> = out.lines().filter(|l| !l.is_empty()).collect();
                    if !lines.is_empty() && lines.iter().all(|l| l.starts_with('-')) {
                        return (Some(true), Some("patch_id"), format!("every commit has an equivalent patch in {r}"));
                    }
                }
            }
            Ok(_) => cherry_reason = Some("has_merge_commits".to_string()),
            Err(_) => cherry_reason = Some("could not list merge commits".to_string()),
        }
    }
    if opts.use_gh
        && let Some(head) = wt.head.as_deref()
            && gh.has_merged_pr(ctx, repo_cwd, branch, head) {
                return (Some(true), Some("gh"), "a merged GitHub PR has this branch at HEAD".into());
            }
    let target = refs.first().map(String::as_str).unwrap_or(default_branch);
    (Some(false), None, cherry_reason.unwrap_or_else(|| format!("not merged into {target}")))
}

fn unpushed_count(wt: &Path, default_branch: &str) -> Result<u32> {
    let has_remotes = git::run(wt, &["remote"]).map(|s| !s.is_empty()).unwrap_or(true);
    let out = if has_remotes {
        git::run(wt, &["rev-list", "--count", "HEAD", "--not", "--remotes"])?
    } else {
        git::run(wt, &["rev-list", "--count", "HEAD", "--not", default_branch])?
    };
    out.trim().parse().map_err(|_| crate::output::CmwError::error(format!("unexpected rev-list output: {out}")))
}

/// Inspect one worktree. Never fails: problems land in `error` or in conservative field values.
pub fn status(ctx: &Ctx, repo_cwd: &Path, wt: &Worktree, default_branch: &str, opts: &StatusOpts, gh: &mut GhCache) -> WorktreeStatus {
    let mut st = WorktreeStatus {
        path: wt.path.clone(),
        branch: wt.branch.clone(),
        head: wt.head.clone(),
        is_main: wt.is_main,
        bare: wt.bare,
        detached: wt.detached,
        locked: wt.locked,
        prunable: wt.prunable,
        dirty: 0,
        has_submodules: false,
        upstream: None,
        ahead: None,
        behind: None,
        unpushed: 0,
        merged: None,
        merged_via: None,
        merge_reason: String::new(),
        last_commit_unix: None,
        idle_days: None,
        hydrated: false,
        apparent_bytes: None,
        error: None,
    };
    if wt.bare {
        st.merge_reason = "bare repository".into();
        return st;
    }
    if !wt.path.is_dir() {
        st.merge_reason = "directory missing".into();
        return st;
    }
    if wt.prunable {
        // The directory exists but git cannot use it (e.g. its `.git` file is gone), so nothing
        // below could be trusted: git commands would see no changes or even an enclosing repo.
        st.merge_reason = "worktree link is broken".into();
        st.error = Some(format!(
            "worktree link is broken (.git file missing or invalid); cmw cannot verify uncommitted work. Repair it with `git worktree repair {}`",
            wt.path.display()
        ));
        return st;
    }

    match git::dirty_count(&wt.path) {
        Ok(n) => st.dirty = n,
        Err(e) => st.error = Some(e.message),
    }
    match git::has_initialized_submodules(&wt.path) {
        Ok(b) => st.has_submodules = b,
        Err(e) => {
            st.error.get_or_insert(e.message);
        }
    }
    st.hydrated = git::git_dir(&wt.path).map(|gd| gd.join(paths::STATE_FILE).exists()).unwrap_or(false);
    (st.last_commit_unix, st.idle_days) = activity(&wt.path);

    if let Ok(up) = git::run(&wt.path, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]) {
        if let Ok(counts) = git::run(&wt.path, &["rev-list", "--left-right", "--count", "HEAD...@{u}"]) {
            let mut it = counts.split_whitespace().map(|n| n.parse::<u32>().ok());
            st.ahead = it.next().flatten();
            st.behind = it.next().flatten();
        }
        st.upstream = Some(up);
    }
    match unpushed_count(&wt.path, default_branch) {
        Ok(n) => st.unpushed = n,
        Err(e) => {
            st.error.get_or_insert(e.message);
        }
    }

    let (merged, via, reason) = detect_merge(ctx, repo_cwd, wt, default_branch, opts, gh);
    st.merged = merged;
    st.merged_via = via.map(str::to_string);
    st.merge_reason = reason;
    if opts.sizes {
        st.apparent_bytes = Some(apparent_size(&wt.path));
    }
    st
}

/// A repository found on disk. `cwd` is any checkout of it, usable to run git.
#[derive(Debug, Clone)]
pub struct RepoRef {
    pub common_dir: PathBuf,
    pub cwd: PathBuf,
}

fn canonical(p: PathBuf) -> PathBuf {
    std::fs::canonicalize(&p).unwrap_or(p)
}

/// Directories (depth <= 3 under `root`) that contain a `.git` FILE, i.e. linked worktrees. Does not descend into them.
fn find_worktree_dirs(root: &Path, out: &mut Vec<PathBuf>) {
    let mut stack = vec![(root.to_path_buf(), 0u8)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            if !ft.is_dir() {
                continue;
            }
            let p = entry.path();
            let dotgit = p.join(".git");
            if std::fs::symlink_metadata(&dotgit).map(|m| m.is_file()).unwrap_or(false) {
                out.push(p);
            } else if depth < 2 {
                stack.push((p, depth + 1));
            }
        }
    }
}

/// The repo of `cwd`, or with `all`, every repo that has a linked worktree under `roots`.
pub fn discover(cwd: &Path, all: bool, roots: &[PathBuf]) -> Result<Vec<RepoRef>> {
    if !all {
        return Ok(vec![RepoRef { common_dir: canonical(git::common_dir(cwd)?), cwd: cwd.to_path_buf() }]);
    }
    let mut dirs = Vec::new();
    for r in roots {
        find_worktree_dirs(r, &mut dirs);
    }
    dirs.sort();
    let mut repos: BTreeMap<PathBuf, PathBuf> = BTreeMap::new();
    for d in dirs {
        if let Ok(common) = git::common_dir(&d) {
            repos.entry(canonical(common)).or_insert(d);
        }
    }
    Ok(repos.into_iter().map(|(common_dir, cwd)| RepoRef { common_dir, cwd }).collect())
}

#[derive(Debug, Serialize)]
pub struct RepoStatus {
    pub repo: String,
    pub common_dir: PathBuf,
    pub default_branch: String,
    pub worktrees: Vec<WorktreeStatus>,
}

#[derive(Debug, Serialize)]
pub struct LsReport {
    pub repos: Vec<RepoStatus>,
    #[serde(skip)]
    sizes: bool,
}

fn table(rows: &[Vec<String>]) -> String {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..cols).map(|c| rows.iter().filter_map(|r| r.get(c)).map(|s| s.chars().count()).max().unwrap_or(0)).collect();
    rows.iter()
        .map(|r| {
            r.iter()
                .enumerate()
                .map(|(i, cell)| if i + 1 == r.len() { cell.clone() } else { format!("{cell:<w$}", w = widths[i]) })
                .collect::<Vec<_>>()
                .join("  ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

impl Report for LsReport {
    fn human(&self) -> String {
        if self.repos.is_empty() {
            return "no worktrees found".into();
        }
        let mut blocks = Vec::new();
        for repo in &self.repos {
            let mut header = vec!["BRANCH", "STATE", "MERGED", "IDLE", "HYDRATED"];
            if self.sizes {
                header.push("SIZE");
            }
            header.push("PATH");
            let mut rows = vec![header.into_iter().map(String::from).collect::<Vec<_>>()];
            for w in &repo.worktrees {
                let mut branch = match (&w.branch, w.bare, w.detached) {
                    (_, true, _) => "(bare)".to_string(),
                    (Some(b), _, _) => b.clone(),
                    (None, _, true) => "(detached)".to_string(),
                    _ => "-".to_string(),
                };
                if w.is_main {
                    branch.push_str(" [main]");
                }
                let state = if w.prunable || !w.path.is_dir() {
                    "missing".to_string()
                } else if w.error.is_some() {
                    "error".to_string()
                } else if w.dirty == 0 {
                    "clean".to_string()
                } else {
                    format!("{} dirty", w.dirty)
                };
                let merged = match w.merged {
                    Some(true) => "yes",
                    Some(false) => "no",
                    None => "?",
                };
                let mut row = vec![
                    branch,
                    state,
                    merged.to_string(),
                    w.idle_days.map_or("-".to_string(), |d| format!("{d}d")),
                    if w.hydrated { "yes" } else { "no" }.to_string(),
                ];
                if self.sizes {
                    row.push(w.apparent_bytes.map_or("-".to_string(), human_bytes));
                }
                row.push(w.path.display().to_string());
                rows.push(row);
            }
            blocks.push(format!("{} ({})\n{}", repo.repo, repo.common_dir.display(), table(&rows)));
        }
        let mut out = blocks.join("\n\n");
        if self.sizes {
            out.push_str("\n\nSIZE is apparent size: APFS clones (node_modules, Pods) are counted at full size, so real disk use is lower.");
        }
        out
    }
}

#[derive(Debug, Serialize)]
pub struct PathReport {
    pub path: PathBuf,
    pub branch: Option<String>,
}
impl Report for PathReport {
    fn human(&self) -> String {
        self.path.display().to_string()
    }
}

pub fn ls(ctx: &Ctx, cwd: &Path, args: &LsArgs) -> Result<LsReport> {
    ls_in(ctx, cwd, args, &paths::scan_roots())
}

fn ls_in(ctx: &Ctx, cwd: &Path, args: &LsArgs, roots: &[PathBuf]) -> Result<LsReport> {
    let repos = discover(cwd, args.all, roots)?;
    // ls stays offline and fast: merge detection uses git only (gc/rm also consult gh).
    let opts = StatusOpts { sizes: args.sizes, use_gh: false };
    let mut out = Vec::new();
    for repo in repos {
        let wts = match git::list_worktrees(&repo.cwd) {
            Ok(w) => w,
            Err(e) => {
                ctx.warn(format!("{}: {}", repo.common_dir.display(), e.message));
                continue;
            }
        };
        let default = git::default_branch(&repo.cwd).unwrap_or_else(|_| "main".to_string());
        let mut gh = GhCache::default();
        let worktrees = wts.iter().map(|w| status(ctx, &repo.cwd, w, &default, &opts, &mut gh)).collect();
        out.push(RepoStatus { repo: git::repo_name(&repo.common_dir), common_dir: repo.common_dir, default_branch: default, worktrees });
    }
    Ok(LsReport { repos: out, sizes: args.sizes })
}

pub fn path(_ctx: &Ctx, cwd: &Path, args: &PathArgs) -> Result<PathReport> {
    let worktree = crate::git::resolve_worktree(cwd, &args.name)?;
    Ok(PathReport { path: worktree.path, branch: worktree.branch })
}

#[cfg(test)]
pub(crate) mod testutil {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    pub fn git(cwd: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// Real repo with one commit on `main`. Returns (tempdir, canonical repo path).
    pub fn repo() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let repo = root.join("repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-b", "main"]);
        git(&repo, &["config", "user.email", "t@example.com"]);
        git(&repo, &["config", "user.name", "T"]);
        git(&repo, &["config", "commit.gpgsign", "false"]);
        std::fs::write(repo.join("README.md"), "hi\n").unwrap();
        std::fs::write(repo.join(".gitignore"), "build/\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "init"]);
        (tmp, repo)
    }

    pub fn commit_file(wt: &Path, name: &str) {
        std::fs::write(wt.join(name), name).unwrap();
        git(wt, &["add", name]);
        git(wt, &["commit", "-m", name]);
    }

    pub fn add_worktree(repo: &Path, name: &str) -> PathBuf {
        let p = repo.parent().unwrap().join("wts").join(name);
        git(repo, &["worktree", "add", "-b", name, p.to_str().unwrap()]);
        std::fs::canonicalize(&p).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;

    fn opts() -> StatusOpts {
        StatusOpts { sizes: false, use_gh: false }
    }

    fn st_of(repo: &Path, wt_path: &Path) -> WorktreeStatus {
        let wts = git::list_worktrees(repo).unwrap();
        let wt = wts.iter().find(|w| w.path == wt_path).unwrap();
        status(&Ctx::new(true, "ls"), repo, wt, "main", &opts(), &mut GhCache::default())
    }

    #[test]
    fn fresh_branch_is_not_merged() {
        let (_t, repo) = repo();
        let wt = add_worktree(&repo, "fresh");
        let st = st_of(&repo, &wt);
        assert_eq!(st.merged, Some(false));
        assert_eq!(st.merge_reason, "no_commits");
        assert_eq!(st.unpushed, 0);
        assert_eq!(st.dirty, 0);
    }

    #[test]
    fn branch_merged_into_main_is_detected() {
        let (_t, repo) = repo();
        let wt = add_worktree(&repo, "feat");
        commit_file(&wt, "a.txt");
        assert_eq!(st_of(&repo, &wt).merged, Some(false));
        git(&repo, &["merge", "--no-edit", "feat"]);
        let st = st_of(&repo, &wt);
        assert_eq!(st.merged, Some(true));
        assert_eq!(st.merged_via.as_deref(), Some("ancestor"));
    }

    #[test]
    fn squash_merge_is_detected_via_patch_id() {
        let (_t, repo) = repo();
        let wt = add_worktree(&repo, "sq");
        commit_file(&wt, "a.txt");
        git(&repo, &["cherry-pick", "sq"]);
        // main moved on so HEAD is no longer simply an ancestor-by-ff, but still equivalent.
        let st = st_of(&repo, &wt);
        assert_eq!(st.merged, Some(true));
    }

    #[test]
    fn main_and_dirty_are_reported() {
        let (_t, repo) = repo();
        let wt = add_worktree(&repo, "d");
        std::fs::write(wt.join("x"), "x").unwrap();
        assert_eq!(st_of(&repo, &wt).dirty, 1);
        let main = st_of(&repo, &repo);
        assert!(main.is_main);
        assert_eq!(main.merged, None);
    }

    #[test]
    fn ls_all_groups_worktrees_by_repo() {
        let (t, repo) = repo();
        add_worktree(&repo, "one");
        add_worktree(&repo, "two");
        let ctx = Ctx::new(true, "ls");
        let roots = vec![t.path().join("wts")];
        let r = ls_in(&ctx, t.path(), &LsArgs { all: true, sizes: true }, &roots).unwrap();
        assert_eq!(r.repos.len(), 1);
        assert_eq!(r.repos[0].worktrees.len(), 3); // main + 2
        assert!(r.repos[0].worktrees.iter().all(|w| w.apparent_bytes.is_some()));
        assert!(r.human().contains("BRANCH"));
    }

    #[test]
    fn existing_but_prunable_worktree_reports_error() {
        let (_t, repo) = repo();
        let wt = add_worktree(&repo, "broken");
        std::fs::write(wt.join("precious"), "x").unwrap();
        std::fs::remove_file(wt.join(".git")).unwrap();
        let st = st_of(&repo, &wt);
        assert!(st.prunable);
        let e = st.error.expect("must not look clean");
        assert!(e.contains("worktree link is broken"), "{e}");
    }

    #[test]
    fn merge_commit_content_defeats_patch_id_proof() {
        let (_t, repo) = repo();
        let wt = add_worktree(&repo, "mc");
        commit_file(&wt, "a.txt");
        git(&wt, &["branch", "side", "main"]);
        git(&wt, &["switch", "-q", "side"]);
        commit_file(&wt, "s.txt");
        git(&wt, &["switch", "-q", "mc"]);
        git(&wt, &["merge", "--no-ff", "--no-commit", "side"]);
        std::fs::write(wt.join("evil.txt"), "unique").unwrap();
        git(&wt, &["add", "evil.txt"]);
        git(&wt, &["commit", "-q", "-m", "merge side"]);
        git(&repo, &["cherry-pick", "mc^1"]); // a.txt
        git(&repo, &["cherry-pick", "side"]);
        let st = st_of(&repo, &wt);
        assert_eq!(st.merged, Some(false));
        assert_eq!(st.merge_reason, "has_merge_commits");
    }

    #[test]
    fn dirty_submodule_is_seen_despite_ignore_config() {
        let (t, repo) = repo();
        let sub = t.path().join("subsrc");
        std::fs::create_dir(&sub).unwrap();
        git(&sub, &["init", "-q", "-b", "main"]);
        git(&sub, &["config", "user.email", "t@example.com"]);
        git(&sub, &["config", "user.name", "T"]);
        commit_file(&sub, "f");
        git(&repo, &["-c", "protocol.file.allow=always", "submodule", "add", "-q", sub.to_str().unwrap(), "libs/sub"]);
        git(&repo, &["commit", "-q", "-m", "add sub"]);
        let wt = add_worktree(&repo, "smod");
        git(&wt, &["-c", "protocol.file.allow=always", "submodule", "update", "--init", "-q"]);
        git(&wt, &["config", "diff.ignoreSubmodules", "all"]);
        std::fs::write(wt.join("libs/sub/precious"), "x").unwrap();
        let st = st_of(&repo, &wt);
        assert!(st.has_submodules);
        assert!(st.dirty >= 1, "dirty submodule hidden: {}", st.dirty);
    }
}
