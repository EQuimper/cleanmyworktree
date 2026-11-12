use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

const LONG_ABOUT: &str = "\
cmw makes git worktrees fast to create, cheap on disk, and safe to clean up.
Built for humans and AI agents running many worktrees in parallel.

  - Dependencies (node_modules, Pods, vendor, ...) are copy-on-write cloned from the main
    checkout: ready in seconds, almost no extra disk.
  - .env, local.properties, keystores and other ignored secrets follow every worktree.
  - Node, iOS, Android, Expo/React Native, PHP, Python, Rust out of the box.
  - Cleanup removes merged worktrees plus their DerivedData, .cxx, build caches.

Agent-friendly by design:
  --json or CMW_JSON=1   one JSON document on stdout: {ok, command, data, warnings} or {ok:false, error:{kind,message,hint}}
  never prompts          destructive commands are dry runs unless --yes / --force
  fix hints              every refusal includes the exact command that resolves it
  idempotent             re-running new/hydrate is safe; `cmw new <branch>` returns the existing path
  exit codes             0 ok, 1 error, 2 usage, 3 refused (unsafe), 4 not found
  parallel-safe          a repo-wide lock serializes concurrent cmw runs";

#[derive(Parser, Debug)]
#[command(name = "cmw", version, about = "Fast, disk-cheap git worktrees for humans and AI agents", long_about = LONG_ABOUT)]
pub struct Cli {
    /// Emit a single JSON document on stdout (also: CMW_JSON=1).
    #[arg(long, global = true, env = "CMW_JSON", value_parser = clap::builder::FalseyValueParser::new())]
    pub json: bool,

    /// Run as if started in this directory (like `git -C`).
    #[arg(short = 'C', global = true, value_name = "DIR")]
    pub cwd: Option<PathBuf>,

    /// None opens the interactive TUI on a terminal; elsewhere it prints help (exit 2).
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Create a worktree for <branch> and hydrate it. Prints the path as the last line.
    New(NewArgs),
    /// Bring deps (clone), secrets (copy) and envs (regen) from the main checkout into a worktree.
    Hydrate(HydrateArgs),
    /// List worktrees with branch, dirty state, merge state, age.
    Ls(LsArgs),
    /// Print the path of a worktree (by branch, slug, dir name or path).
    Path(PathArgs),
    /// Remove a worktree, its build caches and (if merged) its branch.
    Rm(RmArgs),
    /// Find and remove merged, clean worktrees. Dry run unless --yes.
    Gc(GcArgs),
    /// Delete regenerable caches (DerivedData, .cxx, build, .next, ...) from idle worktrees. Dry run unless --yes.
    CleanArtifacts(CleanArtifactsArgs),
    /// Manage copied secrets/config (.env, local.properties, ...).
    Env {
        #[command(subcommand)]
        command: EnvCommand,
    },
    /// Print shell integration (adds `cmw cd <name>`). Use: eval "$(cmw shell-init zsh)"
    ShellInit(ShellInitArgs),
    /// Set up coding agents: skills for Claude Code, Codex and Grok, Claude Code hooks, and a setup prompt to paste.
    Agents {
        #[command(subcommand)]
        command: Option<AgentsCommand>,
    },
    /// Install the cmw skill (same as `cmw agents install`) or print an AGENTS.md snippet.
    Skill {
        #[command(subcommand)]
        command: SkillCommand,
    },
    /// Upgrade cmw to the latest release (Homebrew, installer or cargo). --check only reports.
    Update(UpdateArgs),
    /// Claude Code worktree hooks: print the snippet or merge it into settings.json (see also `cmw agents`).
    Hook {
        #[command(subcommand)]
        command: HookCommand,
    },
}

#[derive(Args, Debug)]
pub struct NewArgs {
    /// Branch to check out. Created from --base if it doesn't exist.
    pub branch: String,
    /// Base ref for a new branch. Default: origin/<default-branch> if present, else <default-branch>.
    #[arg(long)]
    pub base: Option<String>,
    /// Where to create the worktree. Default: ~/.cmw/worktrees/<repo>/<branch-slug> ($CMW_HOME overrides ~/.cmw).
    #[arg(long)]
    pub path: Option<PathBuf>,
    /// `git fetch origin` before creating the branch.
    #[arg(long)]
    pub fetch: bool,
    /// Only create the worktree; don't hydrate.
    #[arg(long)]
    pub no_hydrate: bool,
    #[command(flatten)]
    pub install: InstallFlags,
}

#[derive(Args, Debug, Clone, Copy, Default)]
pub struct InstallFlags {
    /// Never run package managers (clone/copy only).
    #[arg(long)]
    pub no_install: bool,
    /// Skip slow installers (pod install); they are reported as `needs_setup`.
    #[arg(long)]
    pub no_slow: bool,
}

#[derive(Args, Debug)]
pub struct HydrateArgs {
    /// Worktree to hydrate. Default: the current one.
    pub path: Option<PathBuf>,
    /// Source checkout. Default: the repo's main checkout.
    #[arg(long)]
    pub from: Option<PathBuf>,
    /// Show what would happen without changing anything.
    #[arg(long)]
    pub dry_run: bool,
    #[command(flatten)]
    pub install: InstallFlags,
}

#[derive(Args, Debug)]
pub struct LsArgs {
    /// List worktrees of every repo found under ~/.cmw/worktrees, ~/.t3/worktrees and $CMW_SCAN_ROOTS.
    #[arg(long)]
    pub all: bool,
    /// Include apparent disk size per worktree (slower; clones are counted at full size).
    #[arg(long)]
    pub sizes: bool,
}

#[derive(Args, Debug)]
pub struct UpdateArgs {
    /// Only report the current and latest versions; change nothing.
    #[arg(long)]
    pub check: bool,
}

#[derive(Args, Debug)]
pub struct PathArgs {
    /// Branch, branch slug, directory name, or path.
    pub name: String,
}

#[derive(Args, Debug)]
pub struct RmArgs {
    /// Branch, branch slug, directory name, or path.
    pub name: String,
    /// Remove even with uncommitted changes or unpushed, unmerged commits (destroys that work).
    #[arg(long)]
    pub force: bool,
    /// Keep the branch even if it is merged.
    #[arg(long)]
    pub keep_branch: bool,
    /// Show what would be removed.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args, Debug)]
pub struct GcArgs {
    /// Actually remove. Without it, gc only reports.
    #[arg(long)]
    pub yes: bool,
    /// Also report worktrees with no commits for this many days (never auto-removed).
    #[arg(long, value_name = "DAYS")]
    pub stale: Option<u64>,
    /// Every repo under the scan roots, not just the current one.
    #[arg(long)]
    pub all: bool,
}

#[derive(Args, Debug)]
pub struct CleanArtifactsArgs {
    /// Only worktrees with no commits or file changes for this many days.
    #[arg(long, value_name = "DAYS", default_value_t = 7)]
    pub idle: u64,
    /// Actually delete. Without it, only reports.
    #[arg(long)]
    pub yes: bool,
    /// Every repo under the scan roots, not just the current one.
    #[arg(long)]
    pub all: bool,
}

#[derive(Subcommand, Debug)]
pub enum EnvCommand {
    /// Re-copy secrets/config from the main checkout. Files changed in the worktree are kept unless --force.
    Sync(EnvSyncArgs),
}

#[derive(Args, Debug)]
pub struct EnvSyncArgs {
    /// Worktree to sync. Default: the current one.
    pub path: Option<PathBuf>,
    /// Overwrite files that differ in the worktree.
    #[arg(long)]
    pub force: bool,
    /// Show what would change.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args, Debug)]
pub struct ShellInitArgs {
    pub shell: Shell,
}

#[derive(ValueEnum, Debug, Clone, Copy)]
pub enum Shell {
    Zsh,
    Bash,
    Fish,
}

#[derive(Subcommand, Debug)]
pub enum SkillCommand {
    /// Install the cmw skill for every detected agent, or only into --dir.
    Install {
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Print agent instructions (SKILL.md, or an AGENTS.md snippet with --agents-md).
    Print {
        #[arg(long)]
        agents_md: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum HookCommand {
    /// Claude Code settings.json hooks that create/remove worktrees through cmw.
    Claude {
        /// Merge the hooks into ~/.claude/settings.json instead of printing them.
        #[arg(long)]
        install: bool,
        /// With --install: use <repo>/.claude/settings.json of the current repo.
        #[arg(long, requires = "install")]
        project: bool,
    },
    /// Hook entry point: reads Claude Code's WorktreeCreate JSON on stdin, prints the worktree path.
    #[command(hide = true)]
    ClaudeCreate,
    /// Hook entry point: reads Claude Code's WorktreeRemove JSON on stdin.
    #[command(hide = true)]
    ClaudeRemove,
}

#[derive(Subcommand, Debug)]
pub enum AgentsCommand {
    /// Install the cmw skill for every detected agent (Claude Code, Codex, Grok).
    Install(AgentsInstallArgs),
    /// Print the setup prompt to paste into a coding agent.
    Prompt {
        /// Copy it to the clipboard (pbcopy) instead of printing it.
        #[arg(long)]
        copy: bool,
    },
}

#[derive(Args, Debug, Default)]
pub struct AgentsInstallArgs {
    /// Install for exactly these agents, even if not detected. Repeatable.
    #[arg(long = "agent", value_enum, value_name = "AGENT")]
    pub agents: Vec<AgentId>,
    /// Also merge the WorktreeCreate/WorktreeRemove hooks into ~/.claude/settings.json.
    #[arg(long)]
    pub claude_hooks: bool,
    /// Only rewrite cmw skill files that already exist and differ; never create (used by `cmw update`).
    #[arg(long)]
    pub refresh: bool,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentId {
    Claude,
    Codex,
    Grok,
}
