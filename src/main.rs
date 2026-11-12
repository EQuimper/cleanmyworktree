mod agents;
mod cli;
mod clipboard;
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
mod tui;
mod update;

use clap::{CommandFactory, FromArgMatches, Parser};
use cli::{AgentsCommand, Cli, Command, EnvCommand, HookCommand, SkillCommand};
use output::{CmwError, Ctx, Report, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long to wait for another cmw holding the repo lock. `CMW_LOCK_TIMEOUT_SECS` overrides 30s.
fn lock_timeout() -> Duration {
    let secs = std::env::var("CMW_LOCK_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    Duration::from_secs(secs)
}

fn main() {
    let cli = Cli::try_parse().unwrap_or_else(|e| exit_on_parse_error(e));
    let Cli { json, cwd, command } = cli;
    let code = match command {
        Some(command) => run(json, cwd, command),
        None if tui::should_launch(json) => match resolve_cwd(cwd.as_deref()) {
            Ok(dir) => tui::run(dir),
            Err(e) => Ctx::new(false, "cmw").emit_error(&e),
        },
        // Not a terminal (agents, pipes, CI) or JSON requested: keep the pre-TUI contract and
        // fail exactly like a missing subcommand did (help on stderr, exit 2).
        None => {
            let parsed = Cli::command()
                .subcommand_required(true)
                .arg_required_else_help(true)
                .try_get_matches()
                .and_then(|m| Cli::from_arg_matches(&m));
            match parsed {
                Err(e) => exit_on_parse_error(e),
                Ok(_) => unreachable!("a subcommand is required here"),
            }
        }
    };
    std::process::exit(code);
}

fn exit_on_parse_error(e: clap::Error) -> ! {
    use clap::error::ErrorKind as K;
    if matches!(
        e.kind(),
        K::DisplayHelp | K::DisplayVersion | K::DisplayHelpOnMissingArgumentOrSubcommand
    ) {
        e.exit();
    }
    if json_requested() {
        let ctx = Ctx::new(true, "cmw");
        let msg = e.render().to_string();
        let first = msg
            .lines()
            .next()
            .unwrap_or("invalid arguments")
            .trim_start_matches("error: ")
            .to_string();
        std::process::exit(ctx.emit_error(&CmwError::usage(first).with_hint("cmw --help")));
    }
    e.exit(); // clap exits 2 for usage errors, matching our contract.
}

/// The effective directory: `-C <dir>` (canonicalized) or the process cwd.
fn resolve_cwd(dir: Option<&Path>) -> Result<PathBuf> {
    match dir {
        Some(d) => std::fs::canonicalize(d)
            .map_err(|e| CmwError::not_found(format!("-C {}: {e}", d.display()))),
        None => std::env::current_dir()
            .map_err(|e| CmwError::error(format!("cannot read current directory: {e}"))),
    }
}

/// Detect --json / CMW_JSON before clap succeeds, so even usage errors are JSON for agents.
fn json_requested() -> bool {
    std::env::args().any(|a| a == "--json")
        || std::env::var("CMW_JSON")
            .is_ok_and(|v| !matches!(v.as_str(), "" | "0" | "false" | "no" | "off"))
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
        Command::Update(_) => "update",
        Command::Agents { command: None } => "agents",
        Command::Agents {
            command: Some(AgentsCommand::Install(_)),
        } => "agents install",
        Command::Agents {
            command: Some(AgentsCommand::Prompt { .. }),
        } => "agents prompt",
        Command::Skill {
            command: SkillCommand::Install { .. },
        } => "skill install",
        Command::Skill {
            command: SkillCommand::Print { .. },
        } => "skill print",
        Command::Hook {
            command: HookCommand::Claude { .. },
        } => "hook claude",
        Command::Hook {
            command: HookCommand::ClaudeCreate,
        } => "hook claude-create",
        Command::Hook {
            command: HookCommand::ClaudeRemove,
        } => "hook claude-remove",
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

fn run(json: bool, cwd_arg: Option<PathBuf>, mut command: Command) -> i32 {
    // Outputs consumed by other programs keep their raw format even when CMW_JSON=1 is exported:
    // `eval "$(cmw shell-init zsh)"` needs a script, Claude Code's WorktreeCreate hook needs a bare path.
    let raw_output = matches!(
        &command,
        Command::ShellInit(_)
            | Command::Hook {
                command: HookCommand::ClaudeCreate | HookCommand::ClaudeRemove
            }
    );
    let ctx = Ctx::new(json && !raw_output, command_name(&command));
    let cwd = match resolve_cwd(cwd_arg.as_deref()) {
        Ok(p) => p,
        Err(e) => return ctx.emit_error(&e),
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
    match &mut command {
        Command::New(a) => abs(&mut a.path),
        Command::Hydrate(a) => {
            abs(&mut a.path);
            abs(&mut a.from);
        }
        Command::Env {
            command: EnvCommand::Sync(a),
        } => abs(&mut a.path),
        Command::Skill {
            command: SkillCommand::Install { dir },
        } => abs(dir),
        _ => {}
    }

    let mutating = match &command {
        Command::New(_) | Command::Rm(_) => true,
        Command::Hydrate(a) => !a.dry_run,
        Command::Gc(a) => a.yes && !a.all,
        Command::CleanArtifacts(a) => a.yes && !a.all,
        Command::Env {
            command: EnvCommand::Sync(a),
        } => !a.dry_run,
        // Hooks lock the repo named in their stdin payload (see skill.rs); `--all` modes lock per repo.
        _ => false,
    };
    // Lock the repo that is actually mutated: an explicit target path may belong to another repo than cwd.
    let lock_target: PathBuf = match &command {
        Command::Hydrate(a) => a.path.clone().unwrap_or_else(|| cwd.clone()),
        Command::Env {
            command: EnvCommand::Sync(a),
        } => a.path.clone().unwrap_or_else(|| cwd.clone()),
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

    match &command {
        Command::New(a) => finish(&ctx, new::run(&ctx, &cwd, a)),
        Command::Hydrate(a) => finish(&ctx, hydrate::run(&ctx, &cwd, a)),
        Command::Ls(a) => finish(&ctx, inventory::ls(&ctx, &cwd, a)),
        Command::Path(a) => finish(&ctx, inventory::path(&ctx, &cwd, a)),
        Command::Rm(a) => finish(&ctx, gc::rm(&ctx, &cwd, a)),
        Command::Gc(a) => finish(&ctx, gc::gc(&ctx, &cwd, a)),
        Command::CleanArtifacts(a) => finish(&ctx, gc::clean_artifacts(&ctx, &cwd, a)),
        Command::Env {
            command: EnvCommand::Sync(a),
        } => finish(&ctx, env::sync(&ctx, &cwd, a)),
        Command::ShellInit(a) => finish(&ctx, Ok(shell::init(a))),
        Command::Update(a) => finish(&ctx, update::run(&ctx, a)),
        Command::Agents { command: None } => finish(&ctx, Ok(agents::status())),
        Command::Agents {
            command: Some(AgentsCommand::Install(a)),
        } => finish(&ctx, agents::install(&ctx, a)),
        Command::Agents {
            command: Some(AgentsCommand::Prompt { copy }),
        } => finish(&ctx, agents::prompt(*copy)),
        Command::Skill {
            command: SkillCommand::Install { dir: Some(dir) },
        } => finish(&ctx, skill::install(&ctx, dir)),
        Command::Skill {
            command: SkillCommand::Install { dir: None },
        } => finish(
            &ctx,
            agents::install(&ctx, &cli::AgentsInstallArgs::default()),
        ),
        Command::Skill {
            command: SkillCommand::Print { agents_md },
        } => finish(&ctx, Ok(skill::print(*agents_md))),
        Command::Hook {
            command: HookCommand::Claude { install: false, .. },
        } => finish(&ctx, Ok(skill::hook_claude())),
        Command::Hook {
            command: HookCommand::Claude { project, .. },
        } => finish(&ctx, agents::install_hooks(&cwd, *project)),
        Command::Hook {
            command: HookCommand::ClaudeCreate,
        } => finish(&ctx, skill::hook_claude_create(&ctx, &cwd)),
        Command::Hook {
            command: HookCommand::ClaudeRemove,
        } => finish(&ctx, skill::hook_claude_remove(&ctx, &cwd)),
    }
}
