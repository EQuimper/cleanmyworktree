//! Hydrate a worktree from the main checkout: clone deps, copy secrets, run installers.
use crate::cli::{HydrateArgs, InstallFlags};
use crate::clone::{self, CloneMethod};
use crate::output::{CmwError, Ctx, Report, Result, sh_quote};
use crate::paths::{self, STATE_FILE};
use crate::rules::{self, Installer, NATIVE_STAMP_FILES, RuleSet, Strategy};
use crate::{config, git};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Serialize)]
pub struct HydrateReport {
    pub target: PathBuf,
    pub source: PathBuf,
    pub dry_run: bool,
    pub ready: bool,
    pub duration_ms: u64,
    pub entries: Vec<EntryReport>,
    /// Repo-relative reclaimable build dirs removed from cloned native projects.
    pub pruned: Vec<String>,
    /// Ignored entries no rule matched (left alone).
    pub unmatched: Vec<String>,
    pub installs: Vec<InstallItem>,
    pub stale_native: Vec<StaleNative>,
    pub needs_setup: Vec<NeedsSetup>,
}

#[derive(Debug, Serialize)]
pub struct EntryReport {
    pub path: String,
    pub strategy: Strategy,
    pub ecosystem: String,
    /// cloned | copied | exists | regen | skipped | skipped_symlink_parent | would_clone | would_copy
    pub action: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<CloneMethod>,
}

#[derive(Debug, Serialize)]
pub struct InstallItem {
    pub lockfile: String,
    pub dir: String,
    pub ecosystem: String,
    pub command: String,
    /// up_to_date | ran | failed | needs_setup | would_run
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StaleNative {
    pub dir: String,
    pub hint: String,
}

#[derive(Debug, Serialize)]
pub struct NeedsSetup {
    pub dir: String,
    pub ecosystem: String,
    /// Exact command that finishes the setup; empty when there is no known one.
    pub command: String,
    pub reason: String,
}

impl Report for HydrateReport {
    fn human(&self) -> String {
        let count = |f: &dyn Fn(&EntryReport) -> bool| self.entries.iter().filter(|e| f(e)).count();
        let cloned = count(&|e| matches!(e.action, "cloned" | "would_clone"));
        let copied = count(&|e| matches!(e.action, "copied" | "would_copy"));
        let skipped = count(&|e| e.action.starts_with("skipped"));
        let secs = self.duration_ms as f64 / 1000.0;
        let verb = if self.dry_run {
            "dry run, would hydrate"
        } else {
            "hydrated"
        };
        let mut line = format!(
            "{verb} {} in {secs:.1}s: {cloned} cloned, {copied} copied, {skipped} skipped",
            self.target.display()
        );
        if !self.installs.is_empty() {
            let mut parts = Vec::new();
            for (status, label) in [
                ("up_to_date", "up to date"),
                ("ran", "ran"),
                ("would_run", "would run"),
                ("failed", "failed"),
                ("needs_setup", "needs setup"),
            ] {
                let n = self.installs.iter().filter(|i| i.status == status).count();
                if n > 0 {
                    parts.push(format!("{n} {label}"));
                }
            }
            line.push_str(&format!("; installs: {}", parts.join(", ")));
        }
        line.push_str(if self.ready { "; ready" } else { "; not ready" });
        for n in &self.needs_setup {
            if n.command.is_empty() {
                line.push_str(&format!(
                    "\n  needs setup: {} ({})",
                    n.dir_or_root(),
                    n.reason
                ));
            } else {
                line.push_str(&format!("\n  needs setup: {}", n.command));
            }
        }
        for s in &self.stale_native {
            line.push_str(&format!("\n  stale native project: {}", s.hint));
        }
        for i in self.installs.iter().filter(|i| i.status == "failed") {
            line.push_str(&format!("\n  install failed: {}", i.command));
        }
        line
    }
}

impl NeedsSetup {
    fn dir_or_root(&self) -> &str {
        if self.dir.is_empty() { "." } else { &self.dir }
    }
}

/// CLI entry: resolve target (args.path or cwd) and source (args.from or main checkout).
pub fn run(ctx: &Ctx, cwd: &Path, args: &HydrateArgs) -> Result<HydrateReport> {
    let target = git::toplevel(args.path.as_deref().unwrap_or(cwd))?;
    let source = match &args.from {
        Some(p) => p.clone(),
        None => {
            git::main_worktree(&target)?
                .ok_or_else(|| {
                    CmwError::not_found("no main checkout found")
                        .with_hint("cmw hydrate --from <checkout>")
                })?
                .path
        }
    };
    if std::fs::canonicalize(&target)? == std::fs::canonicalize(&source)? {
        return Err(CmwError::refused(
            "target is the main checkout",
            "cmw hydrate <worktree-path>",
        ));
    }
    hydrate(ctx, &source, &target, args.dry_run, args.install)
}

/// Library entry used by `new`.
pub fn hydrate(
    ctx: &Ctx,
    source: &Path,
    target: &Path,
    dry_run: bool,
    install: InstallFlags,
) -> Result<HydrateReport> {
    let started = Instant::now();
    let (rules, cfg) = config::ruleset(target)?;
    let mut ready = true;

    let plan = install_plan(target)?;

    let mut entries = Vec::new();
    let mut pruned = Vec::new();
    let mut unmatched = Vec::new();
    let mut native_dirs: Vec<String> = Vec::new();
    let mut needs_setup = Vec::new();

    let mut state = read_state(target);
    // A provenance record only describes a dep dir that exists. Drop records whose dep dir is
    // missing *before* anything can repopulate it (a clone from main below, or an installer) and
    // persist that, so neither `--no-install` nor a crash mid-hydrate can leave a stale claim.
    if !dry_run {
        let before = state.installs.len();
        state
            .installs
            .retain(|key, _| record_dep_present(target, key));
        if state.installs.len() != before {
            write_state(target, source, state.raw.clone(), &state.installs)?;
        }
    }
    // Repo-relative paths populated from the source in this run.
    let mut cloned_now: HashSet<String> = HashSet::new();

    let mut ignored = Vec::new();
    for entry in git::ignored_entries(source)? {
        expand_agent_dirs(&rules, source, entry, &mut ignored);
    }

    for entry in ignored {
        let Some(rule) = rules.classify(&entry, false) else {
            unmatched.push(entry);
            continue;
        };
        let rel = entry.trim_end_matches('/');
        let src = source.join(rel);
        let dst = target.join(rel);
        let is_dir = entry.ends_with('/');
        let mut method = None;
        let action = match rule.strategy {
            Strategy::Clone | Strategy::Copy => {
                if let Some(link) = symlink_ancestor(target, &dst) {
                    ctx.warn(format!(
                        "{rel}: skipped, {} is a symlink (writing through it would leave the worktree)",
                        link.strip_prefix(target).unwrap_or(&link).display()
                    ));
                    "skipped_symlink_parent"
                } else if std::fs::symlink_metadata(&dst).is_ok() {
                    "exists"
                } else if dry_run {
                    if rule.strategy == Strategy::Clone {
                        "would_clone"
                    } else {
                        "would_copy"
                    }
                } else if rule.strategy == Strategy::Clone || is_dir {
                    let m = clone::clone_tree(&src, &dst)?;
                    if m == CloneMethod::Copy {
                        ctx.warn(format!("{rel}: copy-on-write unavailable, copied in full (uses real disk space)"));
                    }
                    method = Some(m);
                    if rule.native_project {
                        prune_native(&rules, &dst, rel, &mut pruned);
                    }
                    cloned_now.insert(rel.to_string());
                    "cloned"
                } else {
                    if std::fs::symlink_metadata(&src).is_ok_and(|m| m.file_type().is_symlink()) {
                        // Copy the link itself; fs::copy would follow it out of the checkout.
                        clone::copy_recursive(&src, &dst)?;
                    } else {
                        clone::copy_file(&src, &dst)?;
                    }
                    "copied"
                }
            }
            Strategy::Regen => {
                if !install.no_install && !regen_covered(&plan, rel) {
                    let (command, reason) = regen_fallback(&rule.ecosystem, rel);
                    let reason = format!("{rel}: {reason}");
                    ctx.warn(format!("needs setup: {reason}"));
                    needs_setup.push(NeedsSetup {
                        dir: parent_of(rel).to_string(),
                        ecosystem: rule.ecosystem.clone(),
                        command,
                        reason,
                    });
                }
                "regen"
            }
            Strategy::Skip => "skipped",
        };
        if rule.native_project {
            native_dirs.push(parent_of(rel).to_string());
        }
        entries.push(EntryReport {
            path: entry,
            strategy: rule.strategy,
            ecosystem: rule.ecosystem.clone(),
            action,
            method,
        });
    }

    let mut installs = Vec::new();
    if !install.no_install {
        let raw = &state.raw;
        let persist = |recorded: &BTreeMap<String, String>| {
            write_state(target, source, raw.clone(), recorded)
        };
        let env = InstallEnv {
            ctx,
            source,
            target,
            flags: install,
            dry_run,
            cloned_now: &cloned_now,
            persist: &persist,
        };
        for p in &plan {
            installs.push(run_install(&env, p, &mut state.installs));
        }
        for item in &installs {
            match item.status {
                "failed" => ready = false,
                "needs_setup" => needs_setup.push(NeedsSetup {
                    dir: item.dir.clone(),
                    ecosystem: item.ecosystem.clone(),
                    command: item.command.clone(),
                    reason: item.reason.clone().unwrap_or_default(),
                }),
                _ => {}
            }
        }
    }

    let mut stale_native = Vec::new();
    let mut seen = HashSet::new();
    for p in native_dirs {
        if !seen.insert(p.clone()) {
            continue;
        }
        if stamp_hash(&source.join(&p)) != stamp_hash(&target.join(&p)) {
            let hint = if p.is_empty() {
                "npx expo prebuild --clean".to_string()
            } else {
                format!("cd {} && npx expo prebuild --clean", sh_quote(&p))
            };
            ctx.warn(format!(
                "native project in {} was generated from different app config; run: {hint}",
                if p.is_empty() { "." } else { &p }
            ));
            stale_native.push(StaleNative { dir: p, hint });
        }
    }

    // Post hooks are arbitrary commands and may rewrite dependency dirs. Withdraw all records
    // durably while they run, then restore only those whose dep dir is provably untouched
    // (same inode and mtime). Limitation: edits deep inside a dep dir that leave its top-level
    // mtime unchanged are not detected; hooks are expected not to mutate dependencies.
    let mut held: Vec<(String, String, Option<DirStamp>)> = Vec::new();
    if !dry_run && !cfg.post_hydrate.is_empty() && !state.installs.is_empty() {
        for (k, v) in std::mem::take(&mut state.installs) {
            let stamp = record_dep_rel(&k).and_then(|rel| dir_stamp(&target.join(rel)));
            held.push((k, v, stamp));
        }
        write_state(target, source, state.raw.clone(), &state.installs)?;
    }
    if !dry_run {
        for cmd in &cfg.post_hydrate {
            ctx.progress(format!("post-hydrate: {cmd}"));
            let out = Command::new("sh")
                .args(["-c", cmd])
                .current_dir(target)
                .stdin(Stdio::null())
                .output();
            match out {
                Ok(o) if o.status.success() => {}
                Ok(o) => {
                    ctx.warn(format!(
                        "post-hydrate command failed: {cmd}\n{}",
                        tail(&o.stderr, &o.stdout)
                    ));
                    ready = false;
                }
                Err(e) => {
                    ctx.warn(format!("post-hydrate command could not start: {cmd}: {e}"));
                    ready = false;
                }
            }
        }
    }

    for (k, v, stamp) in held {
        let unchanged = match (&stamp, record_dep_rel(&k)) {
            (Some(before), Some(rel)) => dir_stamp(&target.join(rel)).as_ref() == Some(before),
            // No dep dir (e.g. Gemfile.lock): nothing a hook could have swapped out.
            (None, None) => true,
            _ => false,
        };
        if unchanged {
            state.installs.insert(k, v);
            continue;
        }
        // A hook rewrote this dep dir after the install phase judged it current, so this run's
        // `up_to_date` no longer holds: tell the caller exactly what to re-run.
        if let Some((dir, lockfile)) = k.split_once('|')
            && let Some(installer) = rules::installers().iter().find(|i| i.lockfile == lockfile)
        {
            let planned = PlannedInstall {
                installer,
                dir: dir.to_string(),
            };
            let dep = record_dep_rel(&k).unwrap_or_default();
            ctx.warn(format!(
                "post-hydrate hook changed {dep}; run: {}",
                planned.command()
            ));
            needs_setup.push(NeedsSetup {
                dir: dir.to_string(),
                ecosystem: installer.ecosystem.to_string(),
                command: planned.command(),
                reason: format!("post-hydrate hook changed {dep}"),
            });
        }
    }

    ready = ready && needs_setup.is_empty() && stale_native.is_empty();

    if !dry_run {
        write_state(target, source, state.raw, &state.installs)?;
    }

    Ok(HydrateReport {
        target: target.to_path_buf(),
        source: source.to_path_buf(),
        dry_run,
        ready,
        duration_ms: started.elapsed().as_millis() as u64,
        entries,
        pruned,
        unmatched,
        installs,
        stale_native,
        needs_setup,
    })
}

fn write_state(
    target: &Path,
    source: &Path,
    mut out: serde_json::Map<String, serde_json::Value>,
    installs: &BTreeMap<String, String>,
) -> Result<()> {
    out.insert("version".into(), env!("CARGO_PKG_VERSION").into());
    out.insert(
        "hydrated_at_unix".into(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
            .into(),
    );
    out.insert("source".into(), serde_json::json!(source));
    out.insert("installs".into(), serde_json::json!(installs));
    let path = git::git_dir(target)?.join(STATE_FILE);
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&out).expect("serializable state"),
    )?;
    Ok(())
}

/// Previous `STATE_FILE` contents: unknown fields are preserved on rewrite.
struct HydrateState {
    raw: serde_json::Map<String, serde_json::Value>,
    /// `"<rel dir>|<lockfile>"` -> sha256 of the target lockfile when its deps were last made consistent.
    installs: BTreeMap<String, String>,
}

fn read_state(target: &Path) -> HydrateState {
    let raw: serde_json::Map<String, serde_json::Value> = git::git_dir(target)
        .ok()
        .and_then(|d| std::fs::read(d.join(STATE_FILE)).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let installs = raw
        .get("installs")
        .and_then(|v| v.as_object())
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default();
    HydrateState { raw, installs }
}

/// Push `entry` onto `out`. Ignored agent-config containers (`.claude/`, `.agents/`, `.grok/`) are
/// never cloned wholesale: git collapses them into one entry, which would bypass child rules such
/// as `**/.claude/worktrees` (Skip) and duplicate whole worktrees. They are walked instead, each
/// child classified on its own. Symlinks are never followed: a symlink is a file entry (copied as
/// a link), even if it points at a directory.
fn expand_agent_dirs(rules: &RuleSet, source: &Path, entry: String, out: &mut Vec<String>) {
    let is_agent_clone = entry.ends_with('/')
        && rules
            .classify(&entry, false)
            .is_some_and(|r| r.strategy == Strategy::Clone && r.ecosystem == "agent");
    if !is_agent_clone {
        out.push(entry);
        return;
    }
    let Ok(read) = std::fs::read_dir(source.join(entry.trim_end_matches('/'))) else {
        out.push(entry);
        return;
    };
    let mut children: Vec<_> = read.flatten().collect();
    children.sort_by_key(|e| e.file_name());
    for c in children {
        let is_dir = c.file_type().is_ok_and(|t| t.is_dir()); // false for symlinks
        let name = c.file_name().to_string_lossy().into_owned();
        let child = format!("{entry}{name}{}", if is_dir { "/" } else { "" });
        expand_agent_dirs(rules, source, child, out);
    }
}

/// First existing ancestor of `dst` below `root` (exclusive of both) that is a symlink.
fn symlink_ancestor(root: &Path, dst: &Path) -> Option<PathBuf> {
    let rel = dst.strip_prefix(root).ok()?;
    let mut cur = root.to_path_buf();
    let comps: Vec<_> = rel.components().collect();
    for c in &comps[..comps.len().saturating_sub(1)] {
        cur.push(c);
        match std::fs::symlink_metadata(&cur) {
            Ok(m) if m.file_type().is_symlink() => return Some(cur),
            Ok(_) => {}
            Err(_) => return None,
        }
    }
    None
}

fn parent_of(rel: &str) -> &str {
    rel.rsplit_once('/').map_or("", |(p, _)| p)
}

/// Repo-relative dep dir a provenance key (`<dir>|<lockfile>`) describes, if its installer has one.
fn record_dep_rel(key: &str) -> Option<String> {
    let (dir, lockfile) = key.split_once('|')?;
    let inst = rules::installers()
        .iter()
        .find(|i| i.lockfile == lockfile)?;
    inst.dep_dir.map(|d| join_rel(dir, d))
}

/// Identity of a directory for change detection: device, inode and mtime.
#[derive(PartialEq, Eq)]
struct DirStamp(u64, u64, i64, i64);

fn dir_stamp(path: &Path) -> Option<DirStamp> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::symlink_metadata(path).ok()?;
    Some(DirStamp(m.dev(), m.ino(), m.mtime(), m.mtime_nsec()))
}

/// Whether the dep dir described by a provenance key (`<dir>|<lockfile>`) exists in `target`.
/// Unknown keys and installers without a dep dir are kept only when they can still be meaningful.
fn record_dep_present(target: &Path, key: &str) -> bool {
    let Some((dir, lockfile)) = key.split_once('|') else {
        return false;
    };
    match rules::installers().iter().find(|i| i.lockfile == lockfile) {
        Some(inst) => inst
            .dep_dir
            .is_none_or(|d| target.join(join_rel(dir, d)).exists()),
        None => false,
    }
}

fn join_rel(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

/// Delete reclaimable build dirs inside a freshly cloned Expo/RN native project.
fn prune_native(rules: &RuleSet, clone_root: &Path, rel_root: &str, pruned: &mut Vec<String>) {
    fn walk(rules: &RuleSet, dir: &Path, rel: &str, depth: usize, pruned: &mut Vec<String>) {
        if depth > 4 {
            return;
        }
        let Ok(read) = std::fs::read_dir(dir) else {
            return;
        };
        let mut names: Vec<_> = read.flatten().collect();
        names.sort_by_key(|e| e.file_name());
        for e in names {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_symlink() || !ft.is_dir() {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_string();
            if name == "node_modules" || name == "Pods" {
                continue;
            }
            let child_rel = format!("{rel}/{name}");
            if rules
                .classify(&child_rel, true)
                .is_some_and(|r| r.strategy == Strategy::Skip && r.reclaimable)
            {
                if std::fs::remove_dir_all(e.path()).is_ok() {
                    pruned.push(child_rel);
                }
            } else {
                walk(rules, &e.path(), &child_rel, depth + 1, pruned);
            }
        }
    }
    walk(rules, clone_root, rel_root, 1, pruned);
}

struct PlannedInstall {
    installer: &'static Installer,
    dir: String,
}

impl PlannedInstall {
    fn lockfile_rel(&self) -> String {
        join_rel(&self.dir, self.installer.lockfile)
    }
    fn command(&self) -> String {
        let cmd = self.installer.command.join(" ");
        if self.dir.is_empty() {
            cmd
        } else {
            format!("cd {} && {cmd}", sh_quote(&self.dir))
        }
    }
}

/// Tracked lockfiles in the target, one installer per (dir, dep_dir) in installer list order.
fn install_plan(target: &Path) -> Result<Vec<PlannedInstall>> {
    let raw = git::run_raw(target, &["ls-files", "-z"])?;
    let text = String::from_utf8_lossy(&raw);
    let tracked: HashSet<&str> = text.split('\0').filter(|s| !s.is_empty()).collect();
    let mut dirs: Vec<&str> = tracked
        .iter()
        .filter(|p| {
            let base = p.rsplit('/').next().unwrap_or(p);
            rules::installers().iter().any(|i| i.lockfile == base)
        })
        .map(|p| parent_of(p))
        .collect();
    dirs.sort();
    dirs.dedup();
    let mut plan = Vec::new();
    for dir in dirs {
        let mut seen: HashSet<Option<&str>> = HashSet::new();
        for inst in rules::installers() {
            if tracked.contains(join_rel(dir, inst.lockfile).as_str()) && seen.insert(inst.dep_dir)
            {
                plan.push(PlannedInstall {
                    installer: inst,
                    dir: dir.to_string(),
                });
            }
        }
    }
    Ok(plan)
}

fn regen_covered(plan: &[PlannedInstall], rel: &str) -> bool {
    let (dir, name) = (parent_of(rel), rel.rsplit('/').next().unwrap_or(rel));
    plan.iter()
        .any(|p| p.dir == dir && p.installer.dep_dir == Some(name))
}

fn regen_fallback(ecosystem: &str, rel: &str) -> (String, &'static str) {
    let (dir, name) = (parent_of(rel), rel.rsplit('/').next().unwrap_or(rel));
    let cd = if dir.is_empty() {
        String::new()
    } else {
        format!("cd {} && ", sh_quote(dir))
    };
    match ecosystem {
        "python" => (
            format!("{cd}python3 -m venv {name}"),
            "path-bound environment, no tracked lockfile to restore it",
        ),
        "flutter" => (
            format!("{cd}flutter pub get"),
            "path-bound tooling dir, no tracked lockfile to restore it",
        ),
        _ => (
            String::new(),
            "path-bound directory must be regenerated, no installer found",
        ),
    }
}

struct InstallEnv<'a> {
    ctx: &'a Ctx,
    source: &'a Path,
    target: &'a Path,
    flags: InstallFlags,
    dry_run: bool,
    /// Repo-relative paths populated from the source in this run.
    cloned_now: &'a HashSet<String>,
    /// Persists the provenance map (used to make record removal durable before an install).
    persist: &'a dyn Fn(&BTreeMap<String, String>) -> Result<()>,
}

fn run_install(
    env: &InstallEnv,
    p: &PlannedInstall,
    recorded: &mut BTreeMap<String, String>,
) -> InstallItem {
    let InstallEnv {
        ctx,
        source,
        target,
        flags,
        dry_run,
        cloned_now,
        persist,
    } = *env;
    let inst = p.installer;
    let lock_rel = p.lockfile_rel();
    let mut item = InstallItem {
        lockfile: lock_rel.clone(),
        dir: p.dir.clone(),
        ecosystem: inst.ecosystem.to_string(),
        command: p.command(),
        status: "up_to_date",
        reason: None,
    };
    let dep_ok = inst
        .dep_dir
        .is_none_or(|d| target.join(&p.dir).join(d).exists());
    let key = format!("{}|{}", p.dir, inst.lockfile);
    let current = hash_file(&target.join(&lock_rel));
    // A dep dir populated from the source in this run mirrors the source, whatever an older
    // record said: drop the record and trust only lockfile equality with the source.
    let from_source = inst
        .dep_dir
        .is_some_and(|d| cloned_now.contains(&join_rel(&p.dir, d)));
    if from_source {
        recorded.remove(&key);
    }
    let up_to_date = dep_ok
        && if from_source {
            matches!((hash_file(&source.join(&lock_rel)), &current), (Some(s), Some(c)) if &s == c)
        } else {
            // Without a record the provenance of a pre-existing dep dir is unknown (it may have
            // been built from any lockfile), so an install is required.
            matches!((recorded.get(&key), &current), (Some(rec), Some(cur)) if rec == cur)
        };
    if up_to_date {
        if !dry_run {
            recorded.insert(
                key,
                current.expect("up to date implies a readable lockfile"),
            );
        }
        return item;
    }
    if inst.slow && flags.no_slow {
        item.status = "needs_setup";
        item.reason = Some("slow installer skipped by --no-slow".to_string());
        return item;
    }
    if paths::find_in_path(inst.command[0]).is_none() {
        item.status = "needs_setup";
        let reason = format!("{} not found in PATH", inst.command[0]);
        ctx.warn(format!("{}: {reason}; run: {}", lock_rel, item.command));
        item.reason = Some(reason);
        return item;
    }
    if dry_run {
        item.status = "would_run";
        return item;
    }
    // Drop every record describing this dep dir (including other installers' keys, e.g. an old
    // npm record when pnpm now owns node_modules) durably before the installer can touch the
    // deps: a crash or failure must leave no claim that they match any lockfile.
    let dep_rel = inst.dep_dir.map(|d| join_rel(&p.dir, d));
    recorded.retain(|k, _| k != &key && (dep_rel.is_none() || record_dep_rel(k) != dep_rel));
    if let Err(e) = persist(recorded) {
        // Without a durable record removal, a failed install could leave the old record claiming
        // the (now modified) deps still match. Never mutate deps we can't account for.
        item.status = "failed";
        let reason = format!(
            "could not update hydrate state before installing: {}",
            e.message
        );
        ctx.warn(format!("{}: {reason}", item.command));
        item.reason = Some(reason);
        return item;
    }
    ctx.progress(format!("running: {}", item.command));
    let out = Command::new(inst.command[0])
        .args(&inst.command[1..])
        .current_dir(target.join(&p.dir))
        .stdin(Stdio::null())
        .output();
    match out {
        Ok(o) if o.status.success() => {
            item.status = "ran";
            // Hash after the install: it may legitimately rewrite the lockfile.
            if let Some(h) = hash_file(&target.join(&lock_rel)) {
                recorded.insert(key, h);
            }
        }
        Ok(o) => {
            item.status = "failed";
            let t = tail(&o.stderr, &o.stdout);
            ctx.warn(format!("install failed: {}\n{t}", item.command));
            item.reason = Some(format!(
                "exit {}",
                o.status
                    .code()
                    .map_or("signal".to_string(), |c| c.to_string())
            ));
        }
        Err(e) => {
            item.status = "failed";
            ctx.warn(format!("install could not start: {}: {e}", item.command));
            item.reason = Some(e.to_string());
        }
    }
    item
}

/// Last ~20 lines of stderr, falling back to stdout when stderr is empty.
fn tail(stderr: &[u8], stdout: &[u8]) -> String {
    let text = String::from_utf8_lossy(if stderr.iter().all(u8::is_ascii_whitespace) {
        stdout
    } else {
        stderr
    });
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(20)..].join("\n")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hash_file(path: &Path) -> Option<String> {
    std::fs::read(path)
        .ok()
        .map(|b| hex(Sha256::digest(&b).as_slice()))
}

/// Hash of the Expo app config files that exist in `dir` (name + content).
fn stamp_hash(dir: &Path) -> String {
    let mut h = Sha256::new();
    for name in NATIVE_STAMP_FILES {
        if let Ok(bytes) = std::fs::read(dir.join(name)) {
            h.update(name.as_bytes());
            h.update([0]);
            h.update(&bytes);
            h.update([0]);
        }
    }
    hex(h.finalize().as_slice())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::ErrorKind;
    use std::fs;
    use tempfile::TempDir;

    pub(crate) fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Repo with `.gitignore` for typical ignored dirs; returns (tmp, main path).
    pub(crate) fn repo(files: &[(&str, &str)]) -> (TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let main = fs::canonicalize(tmp.path()).unwrap().join("main");
        fs::create_dir_all(&main).unwrap();
        git(&main, &["init", "-q", "-b", "main"]);
        git(&main, &["config", "user.email", "t@example.com"]);
        git(&main, &["config", "user.name", "t"]);
        git(&main, &["config", "commit.gpgsign", "false"]);
        fs::write(
            main.join(".gitignore"),
            "node_modules/\n.env\nbuild/\nios/\nandroid/\n",
        )
        .unwrap();
        for (p, c) in files {
            let f = main.join(p);
            fs::create_dir_all(f.parent().unwrap()).unwrap();
            fs::write(f, c).unwrap();
        }
        git(&main, &["add", "-A"]);
        git(&main, &["commit", "-q", "-m", "init"]);
        (tmp, main)
    }

    pub(crate) fn add_wt(main: &Path, tmp: &TempDir, name: &str) -> PathBuf {
        let wt = fs::canonicalize(tmp.path()).unwrap().join(name);
        git(
            main,
            &["worktree", "add", "-q", "-b", name, wt.to_str().unwrap()],
        );
        wt
    }

    fn ctx() -> Ctx {
        Ctx::new(true, "hydrate")
    }

    fn action<'a>(r: &'a HydrateReport, path: &str) -> &'a str {
        r.entries
            .iter()
            .find(|e| e.path == path)
            .unwrap_or_else(|| panic!("no entry {path}"))
            .action
    }

    #[test]
    fn clones_copies_skips_and_is_idempotent() {
        let (tmp, main) = repo(&[("package.json", "{}")]);
        fs::create_dir_all(main.join("node_modules/dep")).unwrap();
        fs::write(main.join("node_modules/dep/i.js"), "x").unwrap();
        fs::write(main.join(".env"), "A=1").unwrap();
        fs::create_dir_all(main.join("build")).unwrap();
        fs::write(main.join("build/o"), "x").unwrap();
        let wt = add_wt(&main, &tmp, "wt");

        let flags = InstallFlags {
            no_install: true,
            no_slow: false,
        };
        let r = hydrate(&ctx(), &main, &wt, false, flags).unwrap();
        assert_eq!(action(&r, "node_modules/"), "cloned");
        assert_eq!(action(&r, ".env"), "copied");
        assert_eq!(action(&r, "build/"), "skipped");
        assert!(r.installs.is_empty());
        assert!(r.ready);
        assert_eq!(
            fs::read_to_string(wt.join("node_modules/dep/i.js")).unwrap(),
            "x"
        );
        assert_eq!(fs::read_to_string(wt.join(".env")).unwrap(), "A=1");
        assert!(!wt.join("build").exists());
        let state = git::git_dir(&wt).unwrap().join(STATE_FILE);
        assert!(
            fs::read_to_string(state)
                .unwrap()
                .contains("hydrated_at_unix")
        );

        let r2 = hydrate(&ctx(), &main, &wt, false, flags).unwrap();
        assert_eq!(action(&r2, "node_modules/"), "exists");
        assert_eq!(action(&r2, ".env"), "exists");
        assert!(r2.human().starts_with("hydrated "));
    }

    #[test]
    fn dry_run_writes_nothing() {
        let (tmp, main) = repo(&[]);
        fs::create_dir_all(main.join("node_modules")).unwrap();
        fs::write(main.join("node_modules/a"), "x").unwrap();
        fs::write(main.join(".env"), "A=1").unwrap();
        let wt = add_wt(&main, &tmp, "wt");
        let r = hydrate(
            &ctx(),
            &main,
            &wt,
            true,
            InstallFlags {
                no_install: true,
                no_slow: false,
            },
        )
        .unwrap();
        assert_eq!(action(&r, "node_modules/"), "would_clone");
        assert_eq!(action(&r, ".env"), "would_copy");
        assert!(!wt.join("node_modules").exists() && !wt.join(".env").exists());
        assert!(!git::git_dir(&wt).unwrap().join(STATE_FILE).exists());
    }

    #[test]
    fn equal_lockfile_is_up_to_date_and_runs_nothing() {
        let (tmp, main) = repo(&[("package-lock.json", "{\"lockfileVersion\":3}")]);
        fs::create_dir_all(main.join("node_modules/dep")).unwrap();
        fs::write(main.join("node_modules/dep/i.js"), "x").unwrap();
        let wt = add_wt(&main, &tmp, "wt");
        let r = hydrate(&ctx(), &main, &wt, false, InstallFlags::default()).unwrap();
        assert_eq!(r.installs.len(), 1);
        assert_eq!(r.installs[0].status, "up_to_date");
        assert_eq!(r.installs[0].lockfile, "package-lock.json");
        assert!(r.needs_setup.is_empty());
        assert!(r.ready);
        assert!(r.human().contains("installs: 1 up to date; ready"));
    }

    #[test]
    fn missing_dep_dir_is_not_up_to_date() {
        let (tmp, main) = repo(&[("Podfile.lock", "x")]);
        let wt = add_wt(&main, &tmp, "wt");
        let flags = InstallFlags {
            no_install: false,
            no_slow: true,
        };
        let r = hydrate(&ctx(), &main, &wt, false, flags).unwrap();
        assert_eq!(r.installs[0].status, "needs_setup");
        assert_eq!(r.needs_setup[0].command, "pod install");
        assert!(!r.ready);
    }

    #[test]
    fn no_install_skips_install_phase() {
        let (tmp, main) = repo(&[("package-lock.json", "{}")]);
        let wt = add_wt(&main, &tmp, "wt");
        let r = hydrate(
            &ctx(),
            &main,
            &wt,
            false,
            InstallFlags {
                no_install: true,
                no_slow: false,
            },
        )
        .unwrap();
        assert!(r.installs.is_empty() && r.needs_setup.is_empty());
    }

    #[test]
    fn native_project_is_pruned_after_clone() {
        let (tmp, main) = repo(&[("app.json", "{\"a\":1}")]);
        fs::create_dir_all(main.join("ios/build/x")).unwrap();
        fs::create_dir_all(main.join("ios/App")).unwrap();
        fs::write(main.join("ios/build/x/o"), "x").unwrap();
        fs::write(main.join("ios/App/Info.plist"), "p").unwrap();
        fs::create_dir_all(main.join("android/app/.cxx")).unwrap();
        fs::write(main.join("android/app/.cxx/c"), "x").unwrap();
        let wt = add_wt(&main, &tmp, "wt");
        let r = hydrate(
            &ctx(),
            &main,
            &wt,
            false,
            InstallFlags {
                no_install: true,
                no_slow: false,
            },
        )
        .unwrap();
        assert_eq!(action(&r, "ios/"), "cloned");
        assert!(wt.join("ios/App/Info.plist").exists());
        assert!(!wt.join("ios/build").exists());
        assert!(!wt.join("android/app/.cxx").exists());
        assert!(r.pruned.contains(&"ios/build".to_string()));
        assert!(r.pruned.contains(&"android/app/.cxx".to_string()));
        assert!(main.join("ios/build/x/o").exists(), "source untouched");
        assert!(r.stale_native.is_empty());
        assert!(r.ready);
    }

    #[test]
    fn stale_native_when_app_config_differs() {
        let (tmp, main) = repo(&[("app.json", "{\"a\":1}")]);
        fs::create_dir_all(main.join("ios")).unwrap();
        fs::write(main.join("ios/f"), "x").unwrap();
        fs::write(main.join("app.json"), "{\"a\":2}").unwrap(); // main differs from committed copy
        let wt = add_wt(&main, &tmp, "wt");
        let r = hydrate(
            &ctx(),
            &main,
            &wt,
            false,
            InstallFlags {
                no_install: true,
                no_slow: false,
            },
        )
        .unwrap();
        assert_eq!(r.stale_native.len(), 1);
        assert_eq!(r.stale_native[0].hint, "npx expo prebuild --clean");
        assert!(!r.ready);
    }

    #[test]
    fn post_hydrate_failure_marks_not_ready() {
        let (tmp, main) = repo(&[(".cmw.toml", "[hydrate]\npost = [\"exit 3\"]\n")]);
        let wt = add_wt(&main, &tmp, "wt");
        let r = hydrate(
            &ctx(),
            &main,
            &wt,
            false,
            InstallFlags {
                no_install: true,
                no_slow: false,
            },
        )
        .unwrap();
        assert!(!r.ready);
    }

    #[test]
    fn refuses_main_checkout() {
        let (_tmp, main) = repo(&[]);
        let args = HydrateArgs {
            path: None,
            from: None,
            dry_run: false,
            install: InstallFlags::default(),
        };
        let e = run(&ctx(), &main, &args).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Refused);
        assert_eq!(e.hint.as_deref(), Some("cmw hydrate <worktree-path>"));
    }

    #[test]
    fn run_resolves_main_as_source() {
        let (tmp, main) = repo(&[]);
        fs::write(main.join(".env"), "A=1").unwrap();
        let wt = add_wt(&main, &tmp, "wt");
        let args = HydrateArgs {
            path: None,
            from: None,
            dry_run: false,
            install: InstallFlags {
                no_install: true,
                no_slow: false,
            },
        };
        let r = run(&ctx(), &wt, &args).unwrap();
        assert_eq!(r.source, main);
        assert!(wt.join(".env").exists());
    }

    fn fake_installer() -> &'static Installer {
        Box::leak(Box::new(Installer {
            ecosystem: "node",
            lockfile: "package-lock.json",
            command: &["sh", "-c", "touch ran-marker"],
            dep_dir: Some("node_modules"),
            slow: false,
        }))
    }

    #[test]
    fn restored_lockfile_after_install_is_not_up_to_date() {
        let (tmp, main) = repo(&[("package-lock.json", "A")]);
        let wt = add_wt(&main, &tmp, "wt");
        fs::create_dir_all(wt.join("node_modules")).unwrap();
        let p = PlannedInstall {
            installer: fake_installer(),
            dir: String::new(),
        };
        let mut rec = BTreeMap::new();
        let cloned: HashSet<String> = ["node_modules".to_string()].into();
        let go = |rec: &mut BTreeMap<String, String>, cloned: &HashSet<String>| {
            run_install(&env(&main, &wt, cloned, &noop), &p, rec).status
        };
        let none = HashSet::new();
        assert_eq!(go(&mut rec, &cloned), "up_to_date");
        fs::write(wt.join("package-lock.json"), "B").unwrap();
        assert_eq!(go(&mut rec, &none), "ran");
        assert_eq!(go(&mut rec, &none), "up_to_date"); // deps now match B
        fs::write(wt.join("package-lock.json"), "A").unwrap(); // equals main again, deps are from B
        assert_eq!(go(&mut rec, &none), "ran");
        assert_eq!(go(&mut rec, &none), "up_to_date");
    }

    fn lock_fixture(lock: &str) -> (TempDir, PathBuf, PathBuf, PlannedInstall) {
        let (tmp, main) = repo(&[("package-lock.json", lock)]);
        let wt = add_wt(&main, &tmp, "wt");
        (
            tmp,
            main,
            wt,
            PlannedInstall {
                installer: fake_installer(),
                dir: String::new(),
            },
        )
    }

    fn scripted(script: &'static str) -> &'static Installer {
        Box::leak(Box::new(Installer {
            ecosystem: "node",
            lockfile: "package-lock.json",
            command: Box::leak(Box::new(["sh", "-c", script])),
            dep_dir: Some("node_modules"),
            slow: false,
        }))
    }

    fn env<'a>(
        main: &'a Path,
        wt: &'a Path,
        cloned: &'a HashSet<String>,
        persist: &'a dyn Fn(&BTreeMap<String, String>) -> Result<()>,
    ) -> InstallEnv<'a> {
        let ctx: &'static Ctx = Box::leak(Box::new(ctx()));
        InstallEnv {
            ctx,
            source: main,
            target: wt,
            flags: InstallFlags::default(),
            dry_run: false,
            cloned_now: cloned,
            persist,
        }
    }

    fn noop(_: &BTreeMap<String, String>) -> Result<()> {
        Ok(())
    }

    #[test]
    fn recloned_deps_drop_stale_provenance() {
        let (_tmp, main, wt, p) = lock_fixture("A");
        fs::create_dir_all(main.join("node_modules")).unwrap();
        fs::write(main.join("node_modules/v"), "A").unwrap();
        // Worktree lock moved to B and "B" was installed and recorded, then deps were deleted.
        fs::write(wt.join("package-lock.json"), "B").unwrap();
        let mut rec = BTreeMap::new();
        rec.insert(
            "|package-lock.json".to_string(),
            hash_file(&wt.join("package-lock.json")).unwrap(),
        );
        // node_modules re-cloned from main (A) in this run.
        clone::clone_tree(&main.join("node_modules"), &wt.join("node_modules")).unwrap();
        let cloned: HashSet<String> = ["node_modules".to_string()].into();
        let r = run_install(&env(&main, &wt, &cloned, &noop), &p, &mut rec);
        assert_eq!(r.status, "ran", "lock B vs deps A must not be up to date");
    }

    #[test]
    fn failed_install_leaves_no_record_and_persists_removal_first() {
        let (_tmp, main, wt, _) = lock_fixture("A");
        fs::create_dir_all(wt.join("node_modules")).unwrap();
        let p = PlannedInstall {
            installer: scripted("exit 1"),
            dir: String::new(),
        };
        let mut rec = BTreeMap::new();
        rec.insert(
            "|package-lock.json".to_string(),
            hash_file(&wt.join("package-lock.json")).unwrap(),
        ); // record A
        fs::write(wt.join("package-lock.json"), "C").unwrap();
        let seen = std::cell::RefCell::new(Vec::new());
        let persist = |r: &BTreeMap<String, String>| {
            seen.borrow_mut().push(r.len());
            Ok(())
        };
        let cloned = HashSet::new();
        let r = run_install(&env(&main, &wt, &cloned, &persist), &p, &mut rec);
        assert_eq!(r.status, "failed");
        assert!(rec.is_empty());
        assert_eq!(
            *seen.borrow(),
            vec![0],
            "record removal persisted before the installer ran"
        );
        // Lock restored to the previously recorded A: the old record must not resurface.
        fs::write(wt.join("package-lock.json"), "A").unwrap();
        let r = run_install(&env(&main, &wt, &cloned, &noop), &p, &mut rec);
        assert_eq!(r.status, "failed");
    }

    #[test]
    fn unknown_provenance_requires_install() {
        let (_tmp, main, wt, p) = lock_fixture("A");
        fs::create_dir_all(wt.join("node_modules")).unwrap(); // pre-existing, lock equals main
        let mut rec = BTreeMap::new();
        let cloned = HashSet::new();
        let r = run_install(&env(&main, &wt, &cloned, &noop), &p, &mut rec);
        assert_eq!(r.status, "ran");
    }

    #[test]
    fn expansion_does_not_write_through_symlinked_parent() {
        let (tmp, main) = repo(&[]);
        fs::write(main.join(".gitignore"), ".claude/\n").unwrap();
        fs::create_dir_all(main.join(".claude")).unwrap();
        fs::write(main.join(".claude/settings.local.json"), "{}").unwrap();
        let wt = add_wt(&main, &tmp, "wt");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, wt.join(".claude")).unwrap();
        let flags = InstallFlags {
            no_install: true,
            no_slow: false,
        };
        let r = hydrate(&ctx(), &main, &wt, false, flags).unwrap();
        assert_eq!(
            fs::read_dir(&outside).unwrap().count(),
            0,
            "nothing written outside the worktree"
        );
        assert_eq!(
            action(&r, ".claude/settings.local.json"),
            "skipped_symlink_parent"
        );
    }

    #[test]
    fn install_provenance_persists_and_preserves_state_fields() {
        let (tmp, main) = repo(&[("package-lock.json", "A")]);
        fs::create_dir_all(main.join("node_modules")).unwrap();
        let wt = add_wt(&main, &tmp, "wt");
        let state_path = git::git_dir(&wt).unwrap().join(STATE_FILE);
        fs::write(&state_path, "{\"custom\": 7}").unwrap();
        let flags = InstallFlags::default();
        hydrate(&ctx(), &main, &wt, false, flags).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
        assert_eq!(v["custom"], 7);
        assert!(v["installs"]["|package-lock.json"].is_string());
    }

    #[test]
    fn ignored_claude_dir_skips_worktrees_but_keeps_config() {
        let (tmp, main) = repo(&[]);
        fs::write(main.join(".gitignore"), ".claude/\n").unwrap();
        fs::create_dir_all(main.join(".claude/worktrees/old")).unwrap();
        fs::write(main.join(".claude/worktrees/old/precious"), "x").unwrap();
        fs::write(main.join(".claude/worktrees/old/.git"), "gitdir: /x").unwrap();
        fs::create_dir_all(main.join(".claude/agents")).unwrap();
        fs::write(main.join(".claude/agents/x.md"), "agent").unwrap();
        fs::write(main.join(".claude/settings.local.json"), "{}").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("agents", main.join(".claude/link")).unwrap();
        let wt = add_wt(&main, &tmp, "wt");
        let flags = InstallFlags {
            no_install: true,
            no_slow: false,
        };
        let r = hydrate(&ctx(), &main, &wt, false, flags).unwrap();
        assert!(!wt.join(".claude/worktrees").exists());
        assert_eq!(
            fs::read_to_string(wt.join(".claude/agents/x.md")).unwrap(),
            "agent"
        );
        assert!(wt.join(".claude/settings.local.json").exists());
        assert_eq!(action(&r, ".claude/worktrees/"), "skipped");
        assert!(r.entries.iter().all(|e| e.path != ".claude/"));
        #[cfg(unix)]
        assert!(
            fs::symlink_metadata(wt.join(".claude/link"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        // Second run is idempotent.
        let r2 = hydrate(&ctx(), &main, &wt, false, flags).unwrap();
        assert_eq!(action(&r2, ".claude/settings.local.json"), "exists");
    }

    #[test]
    fn no_install_reclone_drops_stale_record() {
        let (_tmp, main, wt, _p) = lock_fixture("A");
        fs::create_dir_all(main.join("node_modules")).unwrap();
        fs::write(main.join("node_modules/v"), "A").unwrap();
        // The worktree moved its lock to B, installed and recorded B, then node_modules was deleted.
        fs::write(wt.join("package-lock.json"), "B").unwrap();
        let mut rec = BTreeMap::new();
        rec.insert(
            "|package-lock.json".to_string(),
            hash_file(&wt.join("package-lock.json")).unwrap(),
        );
        write_state(&wt, &main, Default::default(), &rec).unwrap();
        let flags = InstallFlags {
            no_install: true,
            no_slow: false,
        };
        hydrate(&ctx(), &main, &wt, false, flags).unwrap();
        assert_eq!(fs::read_to_string(wt.join("node_modules/v")).unwrap(), "A");
        assert!(
            read_state(&wt).installs.is_empty(),
            "the B record must not survive a re-clone of A deps"
        );
    }

    #[test]
    fn persist_failure_blocks_installer() {
        let (_tmp, main, wt, _) = lock_fixture("A");
        fs::create_dir_all(wt.join("node_modules")).unwrap();
        let marker = wt.join("installer-ran");
        let script: &'static str =
            Box::leak(format!("touch '{}'", marker.display()).into_boxed_str());
        let p = PlannedInstall {
            installer: scripted(script),
            dir: String::new(),
        };
        let mut rec = BTreeMap::new();
        let fail = |_: &BTreeMap<String, String>| Err(CmwError::error("state is read-only"));
        let cloned = HashSet::new();
        let r = run_install(&env(&main, &wt, &cloned, &fail), &p, &mut rec);
        assert_eq!(r.status, "failed");
        assert!(
            !marker.exists(),
            "the installer must not run when record removal can't be persisted"
        );
    }

    #[test]
    fn switching_installer_drops_other_records_for_same_dep_dir() {
        let (_tmp, main, wt, _) = lock_fixture("A");
        fs::create_dir_all(wt.join("node_modules")).unwrap();
        fs::write(wt.join("pnpm-lock.yaml"), "B").unwrap();
        let pnpm: &'static Installer = Box::leak(Box::new(Installer {
            ecosystem: "node",
            lockfile: "pnpm-lock.yaml",
            command: Box::leak(Box::new(["sh", "-c", "true"])),
            dep_dir: Some("node_modules"),
            slow: false,
        }));
        let p = PlannedInstall {
            installer: pnpm,
            dir: String::new(),
        };
        let mut rec = BTreeMap::new();
        rec.insert(
            "|package-lock.json".to_string(),
            hash_file(&wt.join("package-lock.json")).unwrap(),
        );
        let cloned = HashSet::new();
        let r = run_install(&env(&main, &wt, &cloned, &noop), &p, &mut rec);
        assert_eq!(r.status, "ran");
        assert!(
            !rec.contains_key("|package-lock.json"),
            "npm's record must not survive pnpm rewriting node_modules"
        );
        assert!(rec.contains_key("|pnpm-lock.yaml"));
    }

    fn hook_fixture(hook: &str) -> (TempDir, PathBuf, PathBuf) {
        let (tmp, main, wt, _) = lock_fixture("A");
        fs::create_dir_all(wt.join("node_modules")).unwrap();
        fs::write(
            wt.join(crate::config::FILE_NAME),
            format!("[hydrate]\npost = [{hook:?}]\n"),
        )
        .unwrap();
        let mut rec = BTreeMap::new();
        rec.insert(
            "|package-lock.json".to_string(),
            hash_file(&wt.join("package-lock.json")).unwrap(),
        );
        write_state(&wt, &main, Default::default(), &rec).unwrap();
        (tmp, main, wt)
    }

    #[test]
    fn post_hook_replacing_deps_invalidates_record() {
        let (_tmp, main, wt) = hook_fixture("rm -rf node_modules && mkdir node_modules");
        let flags = InstallFlags {
            no_install: true,
            no_slow: false,
        };
        let r = hydrate(&ctx(), &main, &wt, false, flags).unwrap();
        assert!(read_state(&wt).installs.is_empty());
        assert!(
            !r.ready,
            "deps replaced by a hook must not be reported ready"
        );
        assert_eq!(r.needs_setup.len(), 1);
        assert!(r.needs_setup[0].command.contains("npm install"));
    }

    #[test]
    fn harmless_post_hook_keeps_record() {
        let (_tmp, main, wt) = hook_fixture("true");
        let flags = InstallFlags {
            no_install: true,
            no_slow: false,
        };
        hydrate(&ctx(), &main, &wt, false, flags).unwrap();
        assert!(read_state(&wt).installs.contains_key("|package-lock.json"));
    }
}
