//! `cmw new`: create (or reuse) a worktree for a branch, then hydrate it.
use crate::cli::NewArgs;
use crate::hydrate::{self, HydrateReport};
use crate::output::{CmwError, Ctx, Report, Result};
use crate::{git, paths};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub struct NewReport {
    /// Absolute path of the worktree.
    pub path: PathBuf,
    pub branch: String,
    /// False when a worktree for the branch already existed.
    pub created: bool,
    /// Ref the branch was created from; None when the local branch already existed.
    pub base: Option<String>,
    pub hydrate: Option<HydrateReport>,
}

impl Report for NewReport {
    fn human(&self) -> String {
        self.path.display().to_string()
    }
}

fn ref_exists(cwd: &Path, name: &str) -> bool {
    git::run_ok(cwd, &["show-ref", "--verify", "--quiet", name])
}

pub fn run(ctx: &Ctx, cwd: &Path, args: &NewArgs) -> Result<NewReport> {
    let main = git::main_worktree(cwd)?.ok_or_else(|| CmwError::not_found("no main checkout found"))?;
    let common = git::common_dir(cwd)?;
    let branch = args.branch.as_str();

    let existing = git::list_worktrees(cwd)?
        .into_iter()
        .find(|w| w.branch.as_deref() == Some(branch));

    let (path, created, base) = if let Some(wt) = existing {
        (wt.path, false, None)
    } else {
        let path = match &args.path {
            Some(p) if p.is_absolute() => p.clone(),
            Some(p) => cwd.join(p),
            None => paths::worktrees_root().join(git::repo_name(&common)).join(git::slugify(branch)),
        };
        if std::fs::symlink_metadata(&path).is_ok() {
            return Err(CmwError::refused(
                format!("{} already exists and is not a worktree for '{branch}'", path.display()),
                format!("cmw new {} --path <other-dir>", crate::output::sh_quote(branch)),
            ));
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if args.fetch
            && let Err(e) = git::run(&main.path, &["fetch", "origin"])
        {
            ctx.warn(format!("git fetch origin failed: {}", e.message));
        }
        let p = path.to_string_lossy().to_string();
        let remote = format!("origin/{branch}");
        let base = if ref_exists(&main.path, &format!("refs/heads/{branch}")) {
            git::run(&main.path, &["worktree", "add", &p, branch])?;
            None
        } else if ref_exists(&main.path, &format!("refs/remotes/{remote}")) {
            git::run(&main.path, &["worktree", "add", "--track", "-b", branch, &p, &remote])?;
            Some(remote)
        } else {
            let base = match &args.base {
                Some(b) => b.clone(),
                None => {
                    let def = git::default_branch(&main.path)?;
                    let origin_def = format!("origin/{def}");
                    if ref_exists(&main.path, &format!("refs/remotes/{origin_def}")) { origin_def } else { def }
                }
            };
            git::run(&main.path, &["worktree", "add", "-b", branch, &p, &base])?;
            Some(base)
        };
        (path, true, base)
    };

    let mut hydrate_report = None;
    let is_main = std::fs::canonicalize(&path).ok() == std::fs::canonicalize(&main.path).ok();
    if !args.no_hydrate && !is_main {
        match hydrate::hydrate(ctx, &main.path, &path, false, args.install) {
            Ok(r) => {
                ctx.progress(r.human());
                hydrate_report = Some(r);
            }
            Err(e) => ctx.warn(format!(
                "hydrate failed: {}; retry with: cmw hydrate {}",
                e.message,
                crate::output::sh_quote(&path.to_string_lossy())
            )),
        }
    }

    Ok(NewReport { path, branch: branch.to_string(), created, base, hydrate: hydrate_report })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::InstallFlags;
    use std::fs;
    use std::process::Command;

    fn git_cmd(dir: &Path, args: &[&str]) {
        let out = Command::new("git").args(args).current_dir(dir).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    fn args(branch: &str, path: &Path) -> NewArgs {
        NewArgs {
            branch: branch.into(),
            base: None,
            path: Some(path.to_path_buf()),
            fetch: false,
            no_hydrate: false,
            install: InstallFlags { no_install: true, no_slow: false },
        }
    }

    #[test]
    fn creates_then_reuses_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(tmp.path()).unwrap();
        let main = root.join("main");
        fs::create_dir_all(&main).unwrap();
        git_cmd(&main, &["init", "-q", "-b", "main"]);
        git_cmd(&main, &["config", "user.email", "t@example.com"]);
        git_cmd(&main, &["config", "user.name", "t"]);
        git_cmd(&main, &["config", "commit.gpgsign", "false"]);
        fs::write(main.join(".gitignore"), ".env\n").unwrap();
        fs::write(main.join("a"), "a").unwrap();
        git_cmd(&main, &["add", "-A"]);
        git_cmd(&main, &["commit", "-q", "-m", "init"]);
        fs::write(main.join(".env"), "A=1").unwrap();

        let ctx = Ctx::new(true, "new");
        let wt = root.join("out/feat-x");
        let r = run(&ctx, &main, &args("feat/x", &wt)).unwrap();
        assert!(r.created);
        assert_eq!(r.path, wt);
        assert_eq!(r.base.as_deref(), Some("main"));
        assert_eq!(r.human(), wt.to_string_lossy());
        assert_eq!(fs::read_to_string(wt.join(".env")).unwrap(), "A=1");
        assert!(r.hydrate.is_some());
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["path"], wt.to_string_lossy().as_ref());

        let r2 = run(&ctx, &main, &args("feat/x", &root.join("elsewhere"))).unwrap();
        assert!(!r2.created);
        assert_eq!(r2.path, wt);
        assert!(!root.join("elsewhere").exists());

        // Existing branch without a worktree is checked out, not recreated.
        git_cmd(&main, &["branch", "old"]);
        let r3 = run(&ctx, &main, &NewArgs { no_hydrate: true, ..args("old", &root.join("old")) }).unwrap();
        assert!(r3.created && r3.base.is_none() && r3.hydrate.is_none());

        // Occupied path is refused.
        let e = run(&ctx, &main, &args("other", &root.join("old"))).unwrap_err();
        assert_eq!(e.kind, crate::output::ErrorKind::Refused);
        assert_eq!(e.hint.as_deref(), Some("cmw new other --path <other-dir>"));
    }
}
