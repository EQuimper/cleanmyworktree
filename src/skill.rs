//! `cmw skill install|print`, `cmw hook claude|claude-create|claude-remove`.
//!
//! Claude Code hook contract, verified 2026-11-12 against
//! https://code.claude.com/docs/en/hooks (sections "WorktreeCreate" and "WorktreeRemove"):
//! - Events `WorktreeCreate` and `WorktreeRemove`; no matcher support (matcher is ignored).
//! - Common stdin fields: session_id, transcript_path, cwd, hook_event_name.
//! - WorktreeCreate adds `name` (slug, e.g. "feature-auth"). A command hook must print the
//!   absolute worktree path as the LAST non-empty stdout line (no JSON); anything else goes to
//!   stderr. Non-zero exit or no path fails creation. The path must be normalized (no `.`/`..`)
//!   and must not pass through a symlink below the repo root.
//! - WorktreeRemove adds `worktree_path` (absolute). Exit code decides: non-zero fails removal if
//!   the directory still exists. JSON output is discarded. Claude never deletes the branch.
//! - Settings shape: {"hooks": {"<Event>": [{"hooks": [{"type": "command", "command": "..."}]}]}}.
use crate::cli::{InstallFlags, NewArgs, RmArgs};
use crate::output::{CmwError, Ctx, Report, Result};
use serde::Serialize;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub struct TextReport {
    pub text: String,
}
impl Report for TextReport {
    fn human(&self) -> String {
        self.text.clone()
    }
}

#[derive(Debug, Serialize)]
pub struct SkillInstallReport {
    pub path: String,
    /// `created` | `updated` | `unchanged`
    pub status: String,
}
impl Report for SkillInstallReport {
    fn human(&self) -> String {
        format!("{} {}", self.status, self.path)
    }
}

const SKILL_MD: &str = r#"---
name: cmw
description: Use when creating, hydrating, listing, switching or cleaning git worktrees, or when cutting worktree disk usage; cmw makes worktrees fast, copy-on-write and safe to remove.
---

# cmw: git worktrees for agents

## When to use it

Starting isolated work: run `cmw new <branch> --json`. Read `data.path` (the worktree), then check `data.hydrate.ready`. If it is false, run each `data.hydrate.needs_setup[].command` inside that path.

## Commands

- `cmw ls --json`: list worktrees.
- `cmw path <name>`: print a worktree's path.
- `cmw hydrate`: (re)hydrate the current worktree.
- `cmw env sync`: sync .env files with the main worktree.
- `cmw rm <name>`: remove a worktree (refuses unsafe removals).
- `cmw gc`: dry run of cleanup; `cmw gc --yes` to apply.
- `cmw clean-artifacts`: delete build artifacts to free disk.

## JSON envelope

Success: `{ok, command, data, warnings}`. Failure: `{ok: false, error: {kind, message, hint}}`.

## Exit codes

0 ok, 1 error, 2 usage, 3 refused (unsafe), 4 not found.

## Rules

- When refused (exit 3), run `error.hint` only if it is safe.
- Never pass `--force` to `rm` unless the user explicitly asked to discard work.
- Prefer `-C <dir>` over `cd`.
- `CMW_JSON=1` makes every command emit JSON.

## What hydrate does

Clones dependencies copy-on-write, copies .env and secrets, skips build caches, and runs installs only when lockfiles differ from main.
"#;

const AGENTS_MD: &str = r#"## Worktrees (cmw)

- Start isolated work with `cmw new <branch> --json`; use `data.path`, and run `data.hydrate.needs_setup[].command` if `data.hydrate.ready` is false.
- Other commands: `cmw ls`, `cmw path <name>`, `cmw hydrate`, `cmw env sync`, `cmw rm <name>`, `cmw gc` (dry run) / `cmw gc --yes`.
- Output: `{ok, command, data, warnings}` or `{ok: false, error: {kind, message, hint}}`. Exit codes: 0 ok, 1 error, 2 usage, 3 refused, 4 not found.
- On refusal run the `hint` only if safe. Never use `rm --force` unless the user asked to discard work. Prefer `-C <dir>` over cd.
"#;

pub fn print(agents_md: bool) -> TextReport {
    TextReport { text: if agents_md { AGENTS_MD } else { SKILL_MD }.to_string() }
}

fn default_skill_dir() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| CmwError::error("$HOME is not set; pass --dir"))?;
    Ok(PathBuf::from(home).join(".claude/skills/cmw"))
}

pub fn install(_ctx: &Ctx, dir: Option<&Path>) -> Result<SkillInstallReport> {
    let dir = match dir {
        Some(d) => d.to_path_buf(),
        None => default_skill_dir()?,
    };
    install_to(&dir)
}

fn install_to(dir: &Path) -> Result<SkillInstallReport> {
    let file = dir.join("SKILL.md");
    let status = match std::fs::read_to_string(&file) {
        Ok(existing) if existing == SKILL_MD => "unchanged",
        Ok(_) => "updated",
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "created",
        Err(e) => return Err(e.into()),
    };
    if status != "unchanged" {
        std::fs::create_dir_all(dir)?;
        std::fs::write(&file, SKILL_MD)?;
    }
    Ok(SkillInstallReport { path: file.display().to_string(), status: status.into() })
}

pub fn hook_claude() -> TextReport {
    let doc = serde_json::json!({
        "hooks": {
            "WorktreeCreate": [{"hooks": [{"type": "command", "command": "cmw hook claude-create"}]}],
            "WorktreeRemove": [{"hooks": [{"type": "command", "command": "cmw hook claude-remove"}]}],
        }
    });
    TextReport { text: serde_json::to_string_pretty(&doc).expect("static json") }
}

#[derive(Debug, PartialEq, Eq)]
struct CreatePayload {
    name: String,
    cwd: Option<PathBuf>,
}

#[derive(Debug, PartialEq, Eq)]
struct RemovePayload {
    worktree_path: String,
    cwd: Option<PathBuf>,
}

fn field(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).filter(|s| !s.is_empty()).map(str::to_string)
}

fn parse_create(input: &str) -> Result<CreatePayload> {
    let v: serde_json::Value =
        serde_json::from_str(input).map_err(|e| CmwError::usage(format!("invalid WorktreeCreate JSON on stdin: {e}")))?;
    let name = field(&v, "name").ok_or_else(|| CmwError::usage("WorktreeCreate payload has no `name`"))?;
    Ok(CreatePayload { name, cwd: field(&v, "cwd").map(PathBuf::from) })
}

fn parse_remove(input: &str) -> Result<RemovePayload> {
    let v: serde_json::Value =
        serde_json::from_str(input).map_err(|e| CmwError::usage(format!("invalid WorktreeRemove JSON on stdin: {e}")))?;
    let worktree_path =
        field(&v, "worktree_path").ok_or_else(|| CmwError::usage("WorktreeRemove payload has no `worktree_path`"))?;
    Ok(RemovePayload { worktree_path, cwd: field(&v, "cwd").map(PathBuf::from) })
}

fn read_stdin() -> Result<String> {
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s)?;
    Ok(s)
}

/// WorktreeCreate hook: read stdin JSON, create via `crate::new::run`, stdout = worktree path only.
pub fn hook_claude_create(ctx: &Ctx, cwd: &Path) -> Result<TextReport> {
    let p = parse_create(&read_stdin()?)?;
    let args = NewArgs {
        branch: p.name,
        base: None,
        path: None,
        fetch: false,
        no_hydrate: false,
        install: InstallFlags { no_install: false, no_slow: true },
    };
    let dir = p.cwd.unwrap_or_else(|| cwd.to_path_buf());
    let _lock = crate::lock_repo(&dir)?;
    let report = crate::new::run(ctx, &dir, &args)?;
    Ok(TextReport { text: worktree_path_of(&report)? })
}

/// `cmw new` documents `data.path`; read it from the serialized report so this hook only
/// depends on the JSON contract, not on the report's Rust layout.
fn worktree_path_of<T: Serialize>(report: &T) -> Result<String> {
    let v = serde_json::to_value(report).map_err(|e| CmwError::error(e.to_string()))?;
    v.get("path")
        .and_then(|p| p.as_str())
        .map(str::to_string)
        .ok_or_else(|| CmwError::error("cmw new returned no `path`"))
}

/// WorktreeRemove hook: read stdin JSON, remove via `crate::gc::rm` (never --force).
pub fn hook_claude_remove(ctx: &Ctx, cwd: &Path) -> Result<TextReport> {
    let p = parse_remove(&read_stdin()?)?;
    let args = RmArgs { name: p.worktree_path.clone(), force: false, keep_branch: false, dry_run: false };
    let dir = p.cwd.unwrap_or_else(|| cwd.to_path_buf());
    let _lock = crate::lock_repo(&dir)?;
    crate::gc::rm(ctx, &dir, &args)?;
    Ok(TextReport { text: format!("removed {}", p.worktree_path) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_has_frontmatter() {
        assert!(SKILL_MD.starts_with("---\nname: cmw\ndescription: "));
        assert!(print(true).text.starts_with("## Worktrees (cmw)"));
        assert_eq!(print(false).text, SKILL_MD);
    }

    #[test]
    fn install_is_idempotent() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("a/b");
        assert_eq!(install_to(&dir).unwrap().status, "created");
        assert_eq!(install_to(&dir).unwrap().status, "unchanged");
        std::fs::write(dir.join("SKILL.md"), "old").unwrap();
        assert_eq!(install_to(&dir).unwrap().status, "updated");
        assert_eq!(std::fs::read_to_string(dir.join("SKILL.md")).unwrap(), SKILL_MD);
    }

    #[test]
    fn hook_snippet_wires_both_events() {
        let v: serde_json::Value = serde_json::from_str(&hook_claude().text).unwrap();
        assert_eq!(v["hooks"]["WorktreeCreate"][0]["hooks"][0]["command"], "cmw hook claude-create");
        assert_eq!(v["hooks"]["WorktreeRemove"][0]["hooks"][0]["command"], "cmw hook claude-remove");
    }

    #[test]
    fn parses_create_payload() {
        let p = parse_create(r#"{"hook_event_name":"WorktreeCreate","name":"feature-auth","cwd":"/r"}"#).unwrap();
        assert_eq!(p, CreatePayload { name: "feature-auth".into(), cwd: Some("/r".into()) });
        assert!(parse_create(r#"{"cwd":"/r"}"#).is_err());
        assert!(parse_create("nope").is_err());
        assert_eq!(parse_create(r#"{"name":"x"}"#).unwrap().cwd, None);
    }

    #[test]
    fn extracts_path_from_report() {
        assert_eq!(worktree_path_of(&serde_json::json!({"path": "/w/x", "branch": "b"})).unwrap(), "/w/x");
        assert!(worktree_path_of(&serde_json::json!({"branch": "b"})).is_err());
    }

    #[test]
    fn parses_remove_payload() {
        let p = parse_remove(r#"{"worktree_path":"/w/x","cwd":"/r"}"#).unwrap();
        assert_eq!(p.worktree_path, "/w/x");
        assert!(parse_remove(r#"{"name":"x"}"#).is_err());
    }
}
