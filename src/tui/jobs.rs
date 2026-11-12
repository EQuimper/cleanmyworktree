//! Background work. Each job runs on its own thread with a quiet `Ctx` (nothing may touch the
//! terminal while the TUI owns it) and reports back over a channel. Mutating jobs take the same
//! repo lock as the CLI, so a TUI and agents running `cmw` never step on each other.
use crate::cli::{
    CleanArtifactsArgs, EnvSyncArgs, GcArgs, HydrateArgs, InstallFlags, LsArgs, NewArgs, RmArgs,
};
use crate::gc::{CleanArtifactsReport, GcReport, RmReport};
use crate::inventory::LsReport;
use crate::new::NewReport;
use crate::output::{Ctx, Report, Result};
use crate::{env, gc, hydrate, inventory, new};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

pub enum Msg {
    Ls {
        generation: u64,
        res: Result<LsReport>,
    },
    Size {
        generation: u64,
        path: PathBuf,
        bytes: u64,
    },
    GcPreview {
        generation: u64,
        res: Result<GcReport>,
    },
    SlimPreview {
        generation: u64,
        idle: u64,
        res: Result<CleanArtifactsReport>,
    },
    /// A progress or warning line from the running task.
    Progress(String),
    Done(Done),
}

pub enum Done {
    New {
        res: Result<NewReport>,
        warnings: Vec<String>,
    },
    Remove {
        target: Target,
        force: bool,
        res: Result<RmReport>,
        warnings: Vec<String>,
    },
    /// Any other task: a title and the command's human-readable report.
    Text { title: String, res: Result<String> },
}

/// A worktree acted on from the list.
#[derive(Debug, Clone)]
pub struct Target {
    pub path: PathBuf,
    pub branch: Option<String>,
    /// Main checkout of the target's repo: where to go if the target is removed from under us.
    pub repo_main: Option<PathBuf>,
}

impl Target {
    pub fn label(&self) -> String {
        self.branch.clone().unwrap_or_else(|| {
            self.path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.path.display().to_string())
        })
    }
}

fn quiet(command: &'static str) -> Ctx {
    Ctx::new(true, command)
}

fn streaming(command: &'static str, tx: &Sender<Msg>) -> Ctx {
    let tx = tx.clone();
    Ctx::new(true, command).with_sink(move |line| {
        let _ = tx.send(Msg::Progress(line.to_string()));
    })
}

fn spawn(f: impl FnOnce() + Send + 'static) {
    std::thread::spawn(f);
}

pub fn load_ls(tx: &Sender<Msg>, cwd: &Path, all: bool, generation: u64) {
    let (tx, cwd) = (tx.clone(), cwd.to_path_buf());
    spawn(move || {
        let res = inventory::ls(&quiet("ls"), &cwd, &LsArgs { all, sizes: false });
        let _ = tx.send(Msg::Ls { generation, res });
    });
}

/// Apparent sizes, a few directories at a time. `cancel` stops it between directories.
pub fn load_sizes(tx: &Sender<Msg>, paths: Vec<PathBuf>, generation: u64, cancel: Arc<AtomicBool>) {
    let queue = Arc::new(Mutex::new(paths));
    for _ in 0..3 {
        let (tx, queue, cancel) = (tx.clone(), queue.clone(), cancel.clone());
        spawn(move || {
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                let Some(path) = queue.lock().ok().and_then(|mut q| q.pop()) else {
                    return;
                };
                let bytes = inventory::apparent_size(&path);
                if tx
                    .send(Msg::Size {
                        generation,
                        path,
                        bytes,
                    })
                    .is_err()
                {
                    return;
                }
            }
        });
    }
}

pub fn load_gc_preview(tx: &Sender<Msg>, cwd: &Path, all: bool, generation: u64) {
    let (tx, cwd) = (tx.clone(), cwd.to_path_buf());
    spawn(move || {
        let args = GcArgs {
            yes: false,
            stale: None,
            all,
        };
        let res = gc::gc(&quiet("gc"), &cwd, &args);
        let _ = tx.send(Msg::GcPreview { generation, res });
    });
}

pub fn load_slim_preview(tx: &Sender<Msg>, cwd: &Path, all: bool, idle: u64, generation: u64) {
    let (tx, cwd) = (tx.clone(), cwd.to_path_buf());
    spawn(move || {
        let args = CleanArtifactsArgs {
            idle,
            yes: false,
            all,
        };
        let res = gc::clean_artifacts(&quiet("clean-artifacts"), &cwd, &args);
        let _ = tx.send(Msg::SlimPreview {
            generation,
            idle,
            res,
        });
    });
}

/// A report's human text followed by the warnings the command collected.
pub fn with_warnings(text: String, warnings: &[String]) -> String {
    let mut out = text.trim_end().to_string();
    for w in warnings {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("warning: ");
        out.push_str(w);
    }
    out
}

fn text<R: Report>(ctx: &Ctx, res: Result<R>) -> Result<String> {
    res.map(|r| with_warnings(r.human(), &ctx.warnings()))
}

/// Run `f` with the repo lock of `dir` held.
fn locked_text<R: Report>(
    ctx: &Ctx,
    dir: &Path,
    f: impl FnOnce(&Ctx) -> Result<R>,
) -> Result<String> {
    let _lock = crate::lock_repo(dir)?;
    text(ctx, f(ctx))
}

pub fn new_worktree(tx: &Sender<Msg>, cwd: &Path, branch: String, base: Option<String>) {
    let (tx, cwd) = (tx.clone(), cwd.to_path_buf());
    spawn(move || {
        let ctx = streaming("new", &tx);
        let args = NewArgs {
            branch,
            base,
            path: None,
            fetch: false,
            no_hydrate: false,
            install: InstallFlags::default(),
        };
        let res = crate::lock_repo(&cwd).and_then(|_lock| new::run(&ctx, &cwd, &args));
        let warnings = ctx.warnings();
        let _ = tx.send(Msg::Done(Done::New { res, warnings }));
    });
}

pub fn hydrate(tx: &Sender<Msg>, target: Target) {
    let tx = tx.clone();
    spawn(move || {
        let ctx = streaming("hydrate", &tx);
        let args = HydrateArgs {
            path: Some(target.path.clone()),
            from: None,
            dry_run: false,
            install: InstallFlags::default(),
        };
        let res = locked_text(&ctx, &target.path, |ctx| {
            hydrate::run(ctx, &target.path, &args)
        });
        let title = format!("Hydrated {}", target.label());
        let _ = tx.send(Msg::Done(Done::Text { title, res }));
    });
}

pub fn env_sync(tx: &Sender<Msg>, target: Target) {
    let tx = tx.clone();
    spawn(move || {
        let ctx = streaming("env sync", &tx);
        let args = EnvSyncArgs {
            path: Some(target.path.clone()),
            force: false,
            dry_run: false,
        };
        let res = locked_text(&ctx, &target.path, |ctx| {
            env::sync(ctx, &target.path, &args)
        });
        let title = format!("Synced env for {}", target.label());
        let _ = tx.send(Msg::Done(Done::Text { title, res }));
    });
}

pub fn remove(tx: &Sender<Msg>, target: Target, force: bool) {
    let tx = tx.clone();
    spawn(move || {
        let ctx = streaming("rm", &tx);
        let args = RmArgs {
            name: target.path.display().to_string(),
            force,
            keep_branch: false,
            dry_run: false,
        };
        // Any checkout of the repo works as cwd; the worktree itself is always one.
        let res =
            crate::lock_repo(&target.path).and_then(|_lock| gc::rm(&ctx, &target.path, &args));
        let warnings = ctx.warnings();
        let _ = tx.send(Msg::Done(Done::Remove {
            target,
            force,
            res,
            warnings,
        }));
    });
}

/// Remove the merged worktrees the user confirmed, and only those (see `gc::gc_picked`).
pub fn clean(tx: &Sender<Msg>, picked: Vec<Target>) {
    let tx = tx.clone();
    spawn(move || {
        let ctx = streaming("gc", &tx);
        let paths: Vec<PathBuf> = picked.iter().map(|t| t.path.clone()).collect();
        let lines: Vec<String> = gc::gc_picked(&ctx, &paths)
            .into_iter()
            .map(|(path, res)| match res {
                Ok(r) => r.human(),
                Err(e) => {
                    let label = picked
                        .iter()
                        .find(|t| t.path == path)
                        .map_or_else(|| path.display().to_string(), Target::label);
                    format!("kept {label}: {}", e.message)
                }
            })
            .collect();
        let _ = tx.send(Msg::Done(Done::Text {
            title: "Cleaned up".into(),
            res: Ok(with_warnings(lines.join("\n"), &ctx.warnings())),
        }));
    });
}

pub fn slim(tx: &Sender<Msg>, cwd: &Path, all: bool, idle: u64) {
    let (tx, cwd) = (tx.clone(), cwd.to_path_buf());
    spawn(move || {
        let ctx = streaming("clean-artifacts", &tx);
        let args = CleanArtifactsArgs {
            idle,
            yes: true,
            all,
        };
        let res = if all {
            text(&ctx, gc::clean_artifacts(&ctx, &cwd, &args))
        } else {
            locked_text(&ctx, &cwd, |ctx| gc::clean_artifacts(ctx, &cwd, &args))
        };
        let _ = tx.send(Msg::Done(Done::Text {
            title: "Slimmed down".into(),
            res,
        }));
    });
}
