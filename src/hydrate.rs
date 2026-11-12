//! Hydrate a worktree from the main checkout: clone deps, copy secrets, run installers.
use crate::cli::{HydrateArgs, InstallFlags};
use crate::clone::{self, CloneMethod};
use crate::output::{CmwError, Ctx, Report, Result, sh_quote};
use crate::paths::{self, STATE_FILE};
use crate::rules::{self, Installer, NATIVE_STAMP_FILES, RuleSet, Strategy};
use crate::{config, git};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
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
    /// cloned | copied | exists | regen | skipped | would_clone | would_copy
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
        let skipped = count(&|e| e.action == "skipped");
        let secs = self.duration_ms as f64 / 1000.0;
        let verb = if self.dry_run { "dry run, would hydrate" } else { "hydrated" };
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
                line.push_str(&format!("\n  needs setup: {} ({})", n.dir_or_root(), n.reason));
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
        None => git::main_worktree(cwd)?
            .ok_or_else(|| CmwError::not_found("no main checkout found").with_hint("cmw hydrate --from <checkout>"))?
            .path,
    };
    if std::fs::canonicalize(&target)? == std::fs::canonicalize(&source)? {
        return Err(CmwError::refused("target is the main checkout", "cmw hydrate <worktree-path>"));
    }
    hydrate(ctx, &source, &target, args.dry_run, args.install)
}

/// Library entry used by `new`.
pub fn hydrate(ctx: &Ctx, source: &Path, target: &Path, dry_run: bool, install: InstallFlags) -> Result<HydrateReport> {
    let started = Instant::now();
    let (rules, cfg) = config::ruleset(target)?;
    let mut ready = true;

    let plan = install_plan(target)?;

    let mut entries = Vec::new();
    let mut pruned = Vec::new();
    let mut unmatched = Vec::new();
    let mut native_dirs: Vec<String> = Vec::new();
    let mut needs_setup = Vec::new();

    for entry in git::ignored_entries(source)? {
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
                if std::fs::symlink_metadata(&dst).is_ok() {
                    "exists"
                } else if dry_run {
                    if rule.strategy == Strategy::Clone { "would_clone" } else { "would_copy" }
                } else if rule.strategy == Strategy::Clone || is_dir {
                    let m = clone::clone_tree(&src, &dst)?;
                    if m == CloneMethod::Copy {
                        ctx.warn(format!("{rel}: copy-on-write unavailable, copied in full (uses real disk space)"));
                    }
                    method = Some(m);
                    if rule.native_project {
                        prune_native(&rules, &dst, rel, &mut pruned);
                    }
                    "cloned"
                } else {
                    clone::copy_file(&src, &dst)?;
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
        for p in &plan {
            installs.push(run_install(ctx, source, target, p, install, dry_run));
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
                    ctx.warn(format!("post-hydrate command failed: {cmd}\n{}", tail(&o.stderr, &o.stdout)));
                    ready = false;
                }
                Err(e) => {
                    ctx.warn(format!("post-hydrate command could not start: {cmd}: {e}"));
                    ready = false;
                }
            }
        }
    }

    ready = ready && needs_setup.is_empty() && stale_native.is_empty();

    if !dry_run {
        let state = serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "hydrated_at_unix": SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
            "source": source,
        });
        let path = git::git_dir(target)?.join(STATE_FILE);
        std::fs::write(&path, serde_json::to_string_pretty(&state).expect("serializable state"))?;
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

fn parent_of(rel: &str) -> &str {
    rel.rsplit_once('/').map_or("", |(p, _)| p)
}

fn join_rel(dir: &str, name: &str) -> String {
    if dir.is_empty() { name.to_string() } else { format!("{dir}/{name}") }
}

/// Delete reclaimable build dirs inside a freshly cloned Expo/RN native project.
fn prune_native(rules: &RuleSet, clone_root: &Path, rel_root: &str, pruned: &mut Vec<String>) {
    fn walk(rules: &RuleSet, dir: &Path, rel: &str, depth: usize, pruned: &mut Vec<String>) {
        if depth > 4 {
            return;
        }
        let Ok(read) = std::fs::read_dir(dir) else { return };
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
            if rules.classify(&child_rel, true).is_some_and(|r| r.strategy == Strategy::Skip && r.reclaimable) {
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
        if self.dir.is_empty() { cmd } else { format!("cd {} && {cmd}", sh_quote(&self.dir)) }
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
            if tracked.contains(join_rel(dir, inst.lockfile).as_str()) && seen.insert(inst.dep_dir) {
                plan.push(PlannedInstall { installer: inst, dir: dir.to_string() });
            }
        }
    }
    Ok(plan)
}

fn regen_covered(plan: &[PlannedInstall], rel: &str) -> bool {
    let (dir, name) = (parent_of(rel), rel.rsplit('/').next().unwrap_or(rel));
    plan.iter().any(|p| p.dir == dir && p.installer.dep_dir == Some(name))
}

fn regen_fallback(ecosystem: &str, rel: &str) -> (String, &'static str) {
    let (dir, name) = (parent_of(rel), rel.rsplit('/').next().unwrap_or(rel));
    let cd = if dir.is_empty() { String::new() } else { format!("cd {} && ", sh_quote(dir)) };
    match ecosystem {
        "python" => (format!("{cd}python3 -m venv {name}"), "path-bound environment, no tracked lockfile to restore it"),
        "flutter" => (format!("{cd}flutter pub get"), "path-bound tooling dir, no tracked lockfile to restore it"),
        _ => (String::new(), "path-bound directory must be regenerated, no installer found"),
    }
}

fn run_install(
    ctx: &Ctx,
    source: &Path,
    target: &Path,
    p: &PlannedInstall,
    flags: InstallFlags,
    dry_run: bool,
) -> InstallItem {
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
    let dep_ok = inst.dep_dir.is_none_or(|d| target.join(&p.dir).join(d).exists());
    if hash_file(&source.join(&lock_rel)).is_some_and(|h| Some(h) == hash_file(&target.join(&lock_rel))) && dep_ok {
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
    ctx.progress(format!("running: {}", item.command));
    let out = Command::new(inst.command[0])
        .args(&inst.command[1..])
        .current_dir(target.join(&p.dir))
        .stdin(Stdio::null())
        .output();
    match out {
        Ok(o) if o.status.success() => item.status = "ran",
        Ok(o) => {
            item.status = "failed";
            let t = tail(&o.stderr, &o.stdout);
            ctx.warn(format!("install failed: {}\n{t}", item.command));
            item.reason = Some(format!("exit {}", o.status.code().map_or("signal".to_string(), |c| c.to_string())));
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
    let text = String::from_utf8_lossy(if stderr.iter().all(u8::is_ascii_whitespace) { stdout } else { stderr });
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(20)..].join("\n")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hash_file(path: &Path) -> Option<String> {
    std::fs::read(path).ok().map(|b| hex(Sha256::digest(&b).as_slice()))
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
        let out = Command::new("git").args(args).current_dir(dir).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
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
        fs::write(main.join(".gitignore"), "node_modules/\n.env\nbuild/\nios/\nandroid/\n").unwrap();
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
        git(main, &["worktree", "add", "-q", "-b", name, wt.to_str().unwrap()]);
        wt
    }

    fn ctx() -> Ctx {
        Ctx::new(true, "hydrate")
    }

    fn action<'a>(r: &'a HydrateReport, path: &str) -> &'a str {
        r.entries.iter().find(|e| e.path == path).unwrap_or_else(|| panic!("no entry {path}")).action
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

        let flags = InstallFlags { no_install: true, no_slow: false };
        let r = hydrate(&ctx(), &main, &wt, false, flags).unwrap();
        assert_eq!(action(&r, "node_modules/"), "cloned");
        assert_eq!(action(&r, ".env"), "copied");
        assert_eq!(action(&r, "build/"), "skipped");
        assert!(r.installs.is_empty());
        assert!(r.ready);
        assert_eq!(fs::read_to_string(wt.join("node_modules/dep/i.js")).unwrap(), "x");
        assert_eq!(fs::read_to_string(wt.join(".env")).unwrap(), "A=1");
        assert!(!wt.join("build").exists());
        let state = git::git_dir(&wt).unwrap().join(STATE_FILE);
        assert!(fs::read_to_string(state).unwrap().contains("hydrated_at_unix"));

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
        let r = hydrate(&ctx(), &main, &wt, true, InstallFlags { no_install: true, no_slow: false }).unwrap();
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
        let flags = InstallFlags { no_install: false, no_slow: true };
        let r = hydrate(&ctx(), &main, &wt, false, flags).unwrap();
        assert_eq!(r.installs[0].status, "needs_setup");
        assert_eq!(r.needs_setup[0].command, "pod install");
        assert!(!r.ready);
    }

    #[test]
    fn no_install_skips_install_phase() {
        let (tmp, main) = repo(&[("package-lock.json", "{}")]);
        let wt = add_wt(&main, &tmp, "wt");
        let r = hydrate(&ctx(), &main, &wt, false, InstallFlags { no_install: true, no_slow: false }).unwrap();
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
        let r = hydrate(&ctx(), &main, &wt, false, InstallFlags { no_install: true, no_slow: false }).unwrap();
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
        let r = hydrate(&ctx(), &main, &wt, false, InstallFlags { no_install: true, no_slow: false }).unwrap();
        assert_eq!(r.stale_native.len(), 1);
        assert_eq!(r.stale_native[0].hint, "npx expo prebuild --clean");
        assert!(!r.ready);
    }

    #[test]
    fn post_hydrate_failure_marks_not_ready() {
        let (tmp, main) = repo(&[(".cmw.toml", "[hydrate]\npost = [\"exit 3\"]\n")]);
        let wt = add_wt(&main, &tmp, "wt");
        let r = hydrate(&ctx(), &main, &wt, false, InstallFlags { no_install: true, no_slow: false }).unwrap();
        assert!(!r.ready);
    }

    #[test]
    fn refuses_main_checkout() {
        let (_tmp, main) = repo(&[]);
        let args = HydrateArgs { path: None, from: None, dry_run: false, install: InstallFlags::default() };
        let e = run(&ctx(), &main, &args).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Refused);
        assert_eq!(e.hint.as_deref(), Some("cmw hydrate <worktree-path>"));
    }

    #[test]
    fn run_resolves_main_as_source() {
        let (tmp, main) = repo(&[]);
        fs::write(main.join(".env"), "A=1").unwrap();
        let wt = add_wt(&main, &tmp, "wt");
        let args = HydrateArgs { path: None, from: None, dry_run: false, install: InstallFlags { no_install: true, no_slow: false } };
        let r = run(&ctx(), &wt, &args).unwrap();
        assert_eq!(r.source, main);
        assert!(wt.join(".env").exists());
    }
}
