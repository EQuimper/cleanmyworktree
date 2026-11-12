mod cli;
mod clone;
mod config;
mod env;
mod gc;
mod git;
mod hydrate;
mod inventory;
mod lock;
mod new;
mod output;
mod paths;
mod rules;
mod shell;
mod skill;

use clap::Parser;
use cli::{Cli, Command, EnvCommand, HookCommand, SkillCommand};
use output::{CmwError, Ctx, Report, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long to wait for another cmw holding the repo lock. `CMW_LOCK_TIMEOUT_SECS` overrides 30s.
fn lock_timeout() -> Duration {
    let secs = std::env::var("CMW_LOCK_TIMEOUT_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(30);
    Duration::from_secs(secs)
}

fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            use clap::error::ErrorKind as K;
            if matches!(e.kind(), K::DisplayHelp | K::DisplayVersion | K::DisplayHelpOnMissingArgumentOrSubcommand) {
                e.exit();
            }
            if json_requested() {
                let ctx = Ctx::new(true, "cmw");
                let msg = e.render().to_string();
                let first = msg.lines().next().unwrap_or("invalid arguments").trim_start_matches("error: ").to_string();
                std::process::exit(ctx.emit_error(&CmwError::usage(first).with_hint("cmw --help")));
            }
            e.exit(); // clap exits 2 for usage errors, matching our contract.
        }
    };
    std::process::exit(run(cli));
}

/// Detect --json / CMW_JSON before clap succeeds, so even usage errors are JSON for agents.
fn json_requested() -> bool {
    std::env::args().any(|a| a == "--json")
        || std::env::var("CMW_JSON").is_ok_and(|v| !matches!(v.as_str(), "" | "0" | "false" | "no" | "off"))
}

fn command_name(c: &Command) -> &'static str {
    match c {
        Command::New(_) => "new",
        Command::Hydrate(_) => "hydrate",
        Command::Ls(_) => "ls",
        Command::Path(_) => "path",
        Command::Rm(_) => "rm",
        Command::Gc(_) => "gc",
        Command::CleanArtifacts(_) => "clean-artifacts",
        Command::Env { .. } => "env sync",
        Command::ShellInit(_) => "shell-init",
        Command::Skill { command: SkillCommand::Install { .. } } => "skill install",
        Command::Skill { command: SkillCommand::Print { .. } } => "skill print",
        Command::Hook { command: HookCommand::Claude } => "hook claude",
        Command::Hook { command: HookCommand::ClaudeCreate } => "hook claude-create",
        Command::Hook { command: HookCommand::ClaudeRemove } => "hook claude-remove",
    }
}

fn finish<T: Report>(ctx: &Ctx, res: Result<T>) -> i32 {
    match res {
        Ok(report) => {
            ctx.emit(&report);
            0
        }
        Err(e) => ctx.emit_error(&e),
    }
}

/// Take the repo-wide lock of the repository containing `dir`.
pub(crate) fn lock_repo(dir: &Path) -> Result<lock::RepoLock> {
    let common = git::common_dir(dir)?;
    lock::acquire(&common, lock_timeout())
}

fn run(mut cli: Cli) -> i32 {
    // Outputs consumed by other programs keep their raw format even when CMW_JSON=1 is exported:
    // `eval "$(cmw shell-init zsh)"` needs a script, Claude Code's WorktreeCreate hook needs a bare path.
    let raw_output = matches!(
        &cli.command,
        Command::ShellInit(_) | Command::Hook { command: HookCommand::ClaudeCreate | HookCommand::ClaudeRemove }
    );
    let ctx = Ctx::new(cli.json && !raw_output, command_name(&cli.command));
    let cwd: PathBuf = match &cli.cwd {
        Some(d) => match std::fs::canonicalize(d) {
            Ok(p) => p,
            Err(e) => return ctx.emit_error(&CmwError::not_found(format!("-C {}: {e}", d.display()))),
        },
        None => match std::env::current_dir() {
            Ok(p) => p,
            Err(e) => return ctx.emit_error(&CmwError::error(format!("cannot read current directory: {e}"))),
        },
    };

    // Resolve every path argument once against the effective directory (`-C` or cwd), so locking
    // and execution always act on the same target. Modules then only ever see absolute paths.
    let abs = |p: &mut Option<PathBuf>| {
        if let Some(path) = p.as_mut()
            && path.is_relative()
        {
            *path = cwd.join(&*path);
        }
    };
    match &mut cli.command {
        Command::New(a) => abs(&mut a.path),
        Command::Hydrate(a) => {
            abs(&mut a.path);
            abs(&mut a.from);
        }
        Command::Env { command: EnvCommand::Sync(a) } => abs(&mut a.path),
        Command::Skill { command: SkillCommand::Install { dir } } => abs(dir),
        _ => {}
    }

    let mutating = match &cli.command {
        Command::New(_) | Command::Rm(_) => true,
        Command::Hydrate(a) => !a.dry_run,
        Command::Gc(a) => a.yes && !a.all,
        Command::CleanArtifacts(a) => a.yes && !a.all,
        Command::Env { command: EnvCommand::Sync(a) } => !a.dry_run,
        // Hooks lock the repo named in their stdin payload (see skill.rs); `--all` modes lock per repo.
        _ => false,
    };
    // Lock the repo that is actually mutated: an explicit target path may belong to another repo than cwd.
    let lock_target: PathBuf = match &cli.command {
        Command::Hydrate(a) => a.path.clone().unwrap_or_else(|| cwd.clone()),
        Command::Env { command: EnvCommand::Sync(a) } => a.path.clone().unwrap_or_else(|| cwd.clone()),
        _ => cwd.clone(),
    };
    let _lock = if mutating {
        match lock_repo(&lock_target) {
            Ok(l) => Some(l),
            Err(e) => return ctx.emit_error(&e),
        }
    } else {
        None
    };

    match &cli.command {
        Command::New(a) => finish(&ctx, new::run(&ctx, &cwd, a)),
        Command::Hydrate(a) => finish(&ctx, hydrate::run(&ctx, &cwd, a)),
        Command::Ls(a) => finish(&ctx, inventory::ls(&ctx, &cwd, a)),
        Command::Path(a) => finish(&ctx, inventory::path(&ctx, &cwd, a)),
        Command::Rm(a) => finish(&ctx, gc::rm(&ctx, &cwd, a)),
        Command::Gc(a) => finish(&ctx, gc::gc(&ctx, &cwd, a)),
        Command::CleanArtifacts(a) => finish(&ctx, gc::clean_artifacts(&ctx, &cwd, a)),
        Command::Env { command: EnvCommand::Sync(a) } => finish(&ctx, env::sync(&ctx, &cwd, a)),
        Command::ShellInit(a) => finish(&ctx, Ok(shell::init(a))),
        Command::Skill { command: SkillCommand::Install { dir } } => finish(&ctx, skill::install(&ctx, dir.as_deref())),
        Command::Skill { command: SkillCommand::Print { agents_md } } => finish(&ctx, Ok(skill::print(*agents_md))),
        Command::Hook { command: HookCommand::Claude } => finish(&ctx, Ok(skill::hook_claude())),
        Command::Hook { command: HookCommand::ClaudeCreate } => finish(&ctx, skill::hook_claude_create(&ctx, &cwd)),
        Command::Hook { command: HookCommand::ClaudeRemove } => finish(&ctx, skill::hook_claude_remove(&ctx, &cwd)),
    }
}
