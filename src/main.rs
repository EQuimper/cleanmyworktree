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

const LOCK_TIMEOUT: Duration = Duration::from_secs(30);

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
        Command::Hook { .. } => "hook claude",
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

/// Take the repo-wide lock for commands that change worktrees in the current repo.
fn lock_repo(cwd: &Path) -> Result<lock::RepoLock> {
    let common = git::common_dir(cwd)?;
    lock::acquire(&common, LOCK_TIMEOUT)
}

fn run(cli: Cli) -> i32 {
    let ctx = Ctx::new(cli.json, command_name(&cli.command));
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

    let mutating = match &cli.command {
        Command::New(_) | Command::Rm(_) => true,
        Command::Hydrate(a) => !a.dry_run,
        Command::Gc(a) => a.yes && !a.all,
        Command::CleanArtifacts(a) => a.yes && !a.all,
        Command::Env { command: EnvCommand::Sync(a) } => !a.dry_run,
        _ => false,
    };
    let _lock = if mutating {
        match lock_repo(&cwd) {
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
    }
}
