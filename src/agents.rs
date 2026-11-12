//! `cmw agents [install|prompt]`: set up coding agents (skills, Claude Code hooks, setup prompt).
//!
//! Skill locations (verified): Claude Code reads `~/.claude/skills/<name>/SKILL.md`, Codex reads
//! `~/.agents/skills/<name>/SKILL.md`, Grok reads `~/.grok/skills` (and `~/.claude/skills`).
//! T3 Code runs those agents and has no skill directory of its own.
//! Every path derives from `$HOME`, so tests sandbox `HOME`.

use crate::cli::{AgentId, AgentsInstallArgs};
use crate::clipboard;
use crate::output::{CmwError, Ctx, Report, Result, Style};
use crate::paths::{home, tilde};
use crate::skill::{SKILL_MD, install_to};
use serde::Serialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub(crate) const PROMPT_MD: &str = include_str!("../assets/agent/prompt.md");

const CREATE_CMD: &str = "cmw hook claude-create";
const REMOVE_CMD: &str = "cmw hook claude-remove";
const PROMPT_CMD: &str = "cmw agents prompt --copy";

/// sha256 of every SKILL.md a cmw release has installed, oldest first. Only a file matching one of
/// these is replaced by `--refresh` (and so by `cmw update`): a skill someone edited is theirs.
/// When assets/agent/SKILL.md changes, append its new hash; `shipped_hashes_include_current` fails
/// until you do.
const SHIPPED_SKILL_SHA256: &[&str] = &[
    // v0.1.0 – v0.1.3
    "72cc364565fb21586d06995528c47db07f6cffaf7b954e1572d8541939e4f970",
    // v0.1.4: harness-made worktrees, hydrate in place
    "7c053351a1df13707460766411812d452149022944f9a87ebaf286840c497ab5",
];

fn sha256_hex(text: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A file cmw wrote, unchanged since: an older release's skill or the current one.
fn is_shipped(text: &str) -> bool {
    SHIPPED_SKILL_SHA256.contains(&sha256_hex(text).as_str())
}

struct Agent {
    id: &'static str,
    name: &'static str,
    /// Directory under $HOME whose existence means the agent is installed.
    marker: &'static str,
    /// cmw skill directory under $HOME. None: the agent has no skill directory.
    skill_dir: Option<&'static str>,
}

const AGENTS: [Agent; 4] = [
    Agent {
        id: "claude",
        name: "Claude Code",
        marker: ".claude",
        skill_dir: Some(".claude/skills/cmw"),
    },
    Agent {
        id: "codex",
        name: "Codex",
        marker: ".codex",
        skill_dir: Some(".agents/skills/cmw"),
    },
    Agent {
        id: "grok",
        name: "Grok",
        marker: ".grok",
        skill_dir: Some(".grok/skills/cmw"),
    },
    Agent {
        id: "t3",
        name: "T3 Code",
        marker: ".t3",
        skill_dir: None,
    },
];

/// Where `agents install` writes when no agent is detected.
const FALLBACK_SKILL_DIR: &str = ".agents/skills/cmw";

impl AgentId {
    fn id(self) -> &'static str {
        match self {
            AgentId::Claude => "claude",
            AgentId::Codex => "codex",
            AgentId::Grok => "grok",
        }
    }
}

fn agent(id: &str) -> &'static Agent {
    AGENTS.iter().find(|a| a.id == id).expect("known agent id")
}

fn detected(home: &Path, a: &Agent) -> bool {
    home.join(a.marker).is_dir()
}

fn claude_settings(home: &Path) -> PathBuf {
    home.join(".claude/settings.json")
}

// ---------------------------------------------------------------------------------------------
// Status

#[derive(Debug, Serialize)]
pub struct SkillState {
    pub path: String,
    /// `installed` | `outdated` (an older cmw release's skill) | `modified` (edited, kept by
    /// `cmw update`) | `missing`
    pub status: &'static str,
}

#[derive(Debug, Serialize)]
pub struct AgentStatus {
    pub id: &'static str,
    pub name: &'static str,
    pub detected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill: Option<SkillState>,
    /// Claude Code only: `on` | `partial` | `off`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hooks: Option<&'static str>,
    /// T3 Code only: the worktree setup script to use.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_setup: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct StatusReport {
    pub agents: Vec<AgentStatus>,
    pub prompt: &'static str,
}

fn skill_status(file: &Path) -> &'static str {
    match std::fs::read_to_string(file) {
        Ok(s) if s == SKILL_MD => "installed",
        Ok(s) if is_shipped(&s) => "outdated",
        Ok(_) => "modified",
        Err(_) => "missing",
    }
}

pub fn status() -> StatusReport {
    status_in(&home())
}

pub fn status_in(home: &Path) -> StatusReport {
    let agents = AGENTS
        .iter()
        .map(|a| {
            let skill = a.skill_dir.map(|d| {
                let file = home.join(d).join("SKILL.md");
                SkillState {
                    path: file.display().to_string(),
                    status: skill_status(&file),
                }
            });
            AgentStatus {
                id: a.id,
                name: a.name,
                detected: detected(home, a),
                skill,
                hooks: (a.id == "claude").then(|| hooks_state(&claude_settings(home))),
                worktree_setup: (a.id == "t3").then_some("cmw hydrate"),
            }
        })
        .collect();
    StatusReport {
        agents,
        prompt: PROMPT_CMD,
    }
}

impl StatusReport {
    /// Detected agents that have a skill directory.
    pub fn detected_with_skill(&self) -> impl Iterator<Item = &AgentStatus> {
        self.agents
            .iter()
            .filter(|a| a.detected && a.skill.is_some())
    }

    fn needs_install(&self) -> bool {
        self.detected_with_skill().any(|a| {
            a.skill
                .as_ref()
                .is_some_and(|s| matches!(s.status, "missing" | "outdated"))
        })
    }

    pub fn hooks(&self) -> &'static str {
        self.agents.iter().find_map(|a| a.hooks).unwrap_or("off")
    }

    fn render(&self, s: Style) -> String {
        let name_w = self.agents.iter().map(|a| a.name.len()).max().unwrap_or(0);
        let mut out = Vec::new();
        for a in &self.agents {
            let name = format!("{:name_w$}", a.name);
            let detail = if a.worktree_setup.is_some() {
                if !a.detected {
                    s.dim("not found")
                } else {
                    format!("paste `{PROMPT_CMD}` into a T3 thread to set up worktrees")
                }
            } else {
                match &a.skill {
                    Some(sk) if a.detected || sk.status != "missing" => {
                        let paint = match sk.status {
                            "installed" => s.ok(sk.status),
                            "outdated" | "modified" => s.warn(sk.status),
                            _ => s.bad(sk.status),
                        };
                        let mut d = format!("skill {paint}");
                        if let Some(h) = a.hooks {
                            let hp = match h {
                                "on" => s.ok(h),
                                "partial" => s.warn(h),
                                _ => s.dim(h),
                            };
                            d.push_str(&format!("   hooks {hp}"));
                        }
                        d
                    }
                    _ => s.dim("not found"),
                }
            };
            out.push(format!("{name}  {detail}"));
        }
        if self.needs_install() {
            out.push("run: cmw agents install".to_string());
        }
        out.join("\n")
    }
}

impl Report for StatusReport {
    fn human(&self) -> String {
        self.render(Style::PLAIN)
    }
    fn human_styled(&self, style: Style) -> String {
        self.render(style)
    }
}

// ---------------------------------------------------------------------------------------------
// Install

#[derive(Debug, Serialize)]
pub struct SkillResult {
    pub agent: &'static str,
    pub path: String,
    /// `created` | `updated` | `unchanged` | `skipped`
    pub status: String,
    #[serde(skip)]
    name: &'static str,
}

#[derive(Debug, Serialize)]
pub struct HooksResult {
    pub path: String,
    /// `added` | `unchanged`
    pub status: &'static str,
}

impl Report for HooksResult {
    fn human(&self) -> String {
        format!("{} {}", self.status, tilde(Path::new(&self.path)))
    }
}

#[derive(Debug, Serialize)]
pub struct InstallReport {
    pub skills: Vec<SkillResult>,
    pub claude_hooks: Option<HooksResult>,
    #[serde(skip)]
    t3_detected: bool,
}

impl InstallReport {
    fn render(&self, s: Style) -> String {
        let name_w = self
            .skills
            .iter()
            .map(|r| r.name.len())
            .chain(self.claude_hooks.iter().map(|_| "Claude hooks".len()))
            .max()
            .unwrap_or(0);
        let status_w = self
            .skills
            .iter()
            .map(|r| r.status.len())
            .chain(self.claude_hooks.iter().map(|h| h.status.len()))
            .max()
            .unwrap_or(0);
        let paint = |st: &str| {
            let padded = format!("{st:status_w$}");
            match st {
                "created" | "updated" | "added" => s.ok(&padded),
                "skipped" => s.warn(&padded),
                _ => s.dim(&padded),
            }
        };
        let mut out = Vec::new();
        for r in &self.skills {
            out.push(format!(
                "{:name_w$}  {}  {}",
                r.name,
                paint(&r.status),
                s.dim(&tilde(Path::new(&r.path)))
            ));
        }
        if let Some(h) = &self.claude_hooks {
            out.push(format!(
                "{:name_w$}  {}  {}",
                "Claude hooks",
                paint(h.status),
                s.dim(&tilde(Path::new(&h.path)))
            ));
        }
        if self.t3_detected {
            out.push(format!(
                "T3 Code: paste `{PROMPT_CMD}` into a T3 thread to set up worktrees"
            ));
        }
        out.join("\n")
    }
}

impl InstallReport {
    /// One line for a toast: what changed, or that everything was current.
    pub fn summary(&self) -> String {
        let changed: Vec<&str> = self
            .skills
            .iter()
            .filter(|r| matches!(r.status.as_str(), "created" | "updated"))
            .map(|r| r.name)
            .collect();
        if changed.is_empty() {
            "Agent skills are up to date".to_string()
        } else {
            format!("Installed the cmw skill for {}", changed.join(", "))
        }
    }
}

impl Report for InstallReport {
    fn human(&self) -> String {
        self.render(Style::PLAIN)
    }
    fn human_styled(&self, style: Style) -> String {
        self.render(style)
    }
}

pub fn install(_ctx: &Ctx, args: &AgentsInstallArgs) -> Result<InstallReport> {
    install_in(&home(), args)
}

pub fn install_in(home: &Path, args: &AgentsInstallArgs) -> Result<InstallReport> {
    let mut skills = Vec::new();
    if args.refresh {
        let picked =
            |a: &&Agent| args.agents.is_empty() || args.agents.iter().any(|id| id.id() == a.id);
        for a in AGENTS
            .iter()
            .filter(|a| a.skill_dir.is_some())
            .filter(picked)
        {
            let dir = home.join(a.skill_dir.expect("filtered"));
            if let Some(status) = refresh_skill(&dir.join("SKILL.md"))? {
                skills.push(SkillResult {
                    agent: a.id,
                    path: dir.join("SKILL.md").display().to_string(),
                    status: status.into(),
                    name: a.name,
                });
            }
        }
    } else {
        let targets: Vec<(&str, &str, &str)> = if !args.agents.is_empty() {
            let mut seen = Vec::new();
            for id in &args.agents {
                if !seen.contains(id) {
                    seen.push(*id);
                }
            }
            seen.iter()
                .map(|id| {
                    let a = agent(id.id());
                    (a.id, a.name, a.skill_dir.expect("skill agent"))
                })
                .collect()
        } else {
            let found: Vec<_> = AGENTS
                .iter()
                .filter(|a| a.skill_dir.is_some() && detected(home, a))
                .map(|a| (a.id, a.name, a.skill_dir.expect("filtered")))
                .collect();
            if found.is_empty() {
                vec![("agents", "Agents", FALLBACK_SKILL_DIR)]
            } else {
                found
            }
        };
        for (id, name, dir) in targets {
            let r = install_to(&home.join(dir))?;
            skills.push(SkillResult {
                agent: id,
                path: r.path,
                status: r.status,
                name,
            });
        }
    }
    let claude_hooks = if args.claude_hooks {
        Some(merge_claude_hooks(&claude_settings(home))?)
    } else {
        None
    };
    Ok(InstallReport {
        skills,
        claude_hooks,
        t3_detected: detected(home, agent("t3")),
    })
}

/// Replace a skill file that an older cmw release wrote and nobody edited. None: file is missing.
fn refresh_skill(file: &Path) -> Result<Option<&'static str>> {
    let existing = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        // Unreadable (permissions, not UTF-8): not ours to touch.
        Err(_) => return Ok(Some("skipped")),
    };
    if existing == SKILL_MD {
        return Ok(Some("unchanged"));
    }
    if !is_shipped(&existing) {
        return Ok(Some("skipped"));
    }
    std::fs::write(file, SKILL_MD)?;
    Ok(Some("updated"))
}

// ---------------------------------------------------------------------------------------------
// Claude Code hooks

fn has_command(root: &Value, event: &str, cmd: &str) -> bool {
    root.get("hooks")
        .and_then(|h| h.get(event))
        .and_then(Value::as_array)
        .is_some_and(|entries| {
            entries.iter().any(|e| {
                e.get("hooks")
                    .and_then(Value::as_array)
                    .is_some_and(|hs| hs.iter().any(|h| h["command"] == cmd))
            })
        })
}

/// `on` when both cmw hooks are in `settings`, `partial` for one, `off` otherwise (also when the
/// file is missing or invalid).
pub fn hooks_state(settings: &Path) -> &'static str {
    let Ok(text) = std::fs::read_to_string(settings) else {
        return "off";
    };
    let Ok(root) = serde_json::from_str::<Value>(&text) else {
        return "off";
    };
    match (
        has_command(&root, "WorktreeCreate", CREATE_CMD),
        has_command(&root, "WorktreeRemove", REMOVE_CMD),
    ) {
        (true, true) => "on",
        (false, false) => "off",
        _ => "partial",
    }
}

fn settings_error(path: &Path, what: &str) -> CmwError {
    CmwError::error(format!("{}: {what}", path.display())).with_hint(format!(
        "fix or remove {}, then run: cmw hook claude --install",
        crate::output::sh_quote(&path.display().to_string())
    ))
}

/// Add cmw's WorktreeCreate/WorktreeRemove hooks to a Claude Code settings file, leaving
/// everything else (including key order) alone. Writes nothing when both are already there.
/// Claude Code rewrites this file too: if it changes while we merge, start over from the new text.
pub fn merge_claude_hooks(path: &Path) -> Result<HooksResult> {
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
        && std::fs::metadata(path).is_err()
    {
        return Err(settings_error(path, "is a symlink to a missing file"));
    }
    for _ in 0..5 {
        let original = match std::fs::read_to_string(path) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(settings_error(path, &format!("cannot read ({e})"))),
        };
        let Some(text) = merged_settings(path, original.as_deref())? else {
            return Ok(HooksResult {
                path: path.display().to_string(),
                status: "unchanged",
            });
        };
        if write_atomic(path, &text, original.as_deref())? {
            return Ok(HooksResult {
                path: path.display().to_string(),
                status: "added",
            });
        }
    }
    Err(CmwError::error(format!(
        "{} kept changing while cmw tried to add its hooks",
        path.display()
    ))
    .with_hint("cmw hook claude --install"))
}

/// The settings text with both hooks present, or None when nothing needs adding.
fn merged_settings(path: &Path, original: Option<&str>) -> Result<Option<String>> {
    let mut root: Value = match original {
        Some(text) => serde_json::from_str(text)
            .map_err(|e| settings_error(path, &format!("not valid JSON ({e})")))?,
        None => json!({}),
    };
    let Some(obj) = root.as_object_mut() else {
        return Err(settings_error(path, "top level is not a JSON object"));
    };
    let mut changed = false;
    for (event, cmd) in [
        ("WorktreeCreate", CREATE_CMD),
        ("WorktreeRemove", REMOVE_CMD),
    ] {
        let probe = Value::Object(obj.clone());
        if has_command(&probe, event, cmd) {
            continue;
        }
        let hooks = obj.entry("hooks").or_insert_with(|| json!({}));
        let Some(hooks) = hooks.as_object_mut() else {
            return Err(settings_error(path, "`hooks` is not an object"));
        };
        let list = hooks.entry(event).or_insert_with(|| json!([]));
        let Some(list) = list.as_array_mut() else {
            return Err(settings_error(
                path,
                &format!("`hooks.{event}` is not an array"),
            ));
        };
        list.push(json!({"hooks": [{"type": "command", "command": cmd}]}));
        changed = true;
    }
    if !changed {
        return Ok(None);
    }
    let mut text = serde_json::to_string_pretty(&root).expect("json value");
    text.push('\n');
    Ok(Some(text))
}

/// Temp file in the same directory, then rename. A symlinked `path` (dotfiles) is resolved so the
/// link survives, and the existing permissions are kept. Returns false, writing nothing, when the
/// file no longer holds `expected` (None: didn't exist), so the caller can merge again.
fn write_atomic(path: &Path, text: &str, expected: Option<&str>) -> Result<bool> {
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let dir = target.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{name}.cmw-tmp-{}", std::process::id()));
    let res = (|| -> std::io::Result<bool> {
        std::fs::write(&tmp, text)?;
        if let Ok(meta) = std::fs::metadata(&target) {
            std::fs::set_permissions(&tmp, meta.permissions())?;
        }
        // Last check right before the rename keeps the lost-update window to a few microseconds.
        let current = match std::fs::read_to_string(&target) {
            Ok(t) => Some(t),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        if current.as_deref() != expected {
            std::fs::remove_file(&tmp)?;
            return Ok(false);
        }
        std::fs::rename(&tmp, &target)?;
        Ok(true)
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    Ok(res?)
}

/// `cmw hook claude --install [--project]`: settings of `~` or of the repo containing `cwd`.
pub fn install_hooks(cwd: &Path, project: bool) -> Result<HooksResult> {
    let path = if project {
        crate::git::toplevel(cwd)?.join(".claude/settings.json")
    } else {
        claude_settings(&home())
    };
    merge_claude_hooks(&path)
}

// ---------------------------------------------------------------------------------------------
// Prompt

#[derive(Debug, Serialize)]
pub struct PromptReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copied: Option<bool>,
}

impl Report for PromptReport {
    fn human(&self) -> String {
        match &self.text {
            Some(t) => t.clone(),
            None => "copied the setup prompt; paste it into Claude Code, Codex, Grok or T3 Code"
                .to_string(),
        }
    }
}

pub fn prompt(copy: bool) -> Result<PromptReport> {
    if !copy {
        return Ok(PromptReport {
            text: Some(PROMPT_MD.to_string()),
            copied: None,
        });
    }
    if clipboard::copy(PROMPT_MD) {
        Ok(PromptReport {
            text: None,
            copied: Some(true),
        })
    } else {
        Err(
            CmwError::error("couldn't copy to the clipboard (pbcopy failed)")
                .with_hint("cmw agents prompt | pbcopy"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn read(p: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    #[test]
    fn merge_creates_missing_file_and_parents() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join(".claude/settings.json");
        assert_eq!(merge_claude_hooks(&p).unwrap().status, "added");
        let v = read(&p);
        assert_eq!(
            v["hooks"]["WorktreeCreate"][0]["hooks"][0]["command"],
            CREATE_CMD
        );
        assert_eq!(
            v["hooks"]["WorktreeRemove"][0]["hooks"][0]["type"],
            "command"
        );
        assert!(std::fs::read_to_string(&p).unwrap().ends_with("}\n"));
        assert_eq!(merge_claude_hooks(&p).unwrap().status, "unchanged");
    }

    #[test]
    fn merge_into_empty_object() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("settings.json");
        std::fs::write(&p, "{}").unwrap();
        assert_eq!(merge_claude_hooks(&p).unwrap().status, "added");
        assert_eq!(hooks_state(&p), "on");
    }

    #[test]
    fn merge_preserves_other_entries_and_key_order() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("settings.json");
        std::fs::write(
            &p,
            r#"{"zeta":1,"hooks":{"PreToolUse":[{"x":1}],"WorktreeCreate":[{"hooks":[{"type":"command","command":"other"}]}]},"alpha":2}"#,
        )
        .unwrap();
        merge_claude_hooks(&p).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.find("zeta").unwrap() < text.find("hooks").unwrap());
        assert!(text.find("hooks").unwrap() < text.find("alpha").unwrap());
        assert!(text.find("PreToolUse").unwrap() < text.find("WorktreeCreate").unwrap());
        let v = read(&p);
        let create = v["hooks"]["WorktreeCreate"].as_array().unwrap();
        assert_eq!(create.len(), 2);
        assert_eq!(create[0]["hooks"][0]["command"], "other");
        assert_eq!(create[1]["hooks"][0]["command"], CREATE_CMD);
        assert_eq!(v["hooks"]["PreToolUse"][0]["x"], 1);
        assert_eq!(v["hooks"]["WorktreeRemove"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn merge_unchanged_when_present_and_partial_adds_missing_half() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("settings.json");
        let src = r#"{"hooks":{"WorktreeCreate":[{"matcher":"","hooks":[{"type":"command","command":"cmw hook claude-create"}]}]}}"#;
        std::fs::write(&p, src).unwrap();
        assert_eq!(hooks_state(&p), "partial");
        assert_eq!(merge_claude_hooks(&p).unwrap().status, "added");
        assert_eq!(
            read(&p)["hooks"]["WorktreeCreate"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let before = std::fs::read_to_string(&p).unwrap();
        assert_eq!(merge_claude_hooks(&p).unwrap().status, "unchanged");
        assert_eq!(std::fs::read_to_string(&p).unwrap(), before);
    }

    #[test]
    fn merge_rejects_invalid_json_and_non_objects_without_writing() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("settings.json");
        for bad in [
            "{not json",
            "[1,2]",
            r#"{"hooks":[]}"#,
            r#"{"hooks":{"WorktreeCreate":{}}}"#,
        ] {
            std::fs::write(&p, bad).unwrap();
            let e = merge_claude_hooks(&p).unwrap_err();
            assert!(e.hint.is_some(), "{bad}");
            assert_eq!(std::fs::read_to_string(&p).unwrap(), bad);
            assert_eq!(hooks_state(&p), "off");
        }
    }

    #[test]
    fn merge_keeps_symlinked_settings() {
        let t = tempfile::tempdir().unwrap();
        let real = t.path().join("dotfiles/settings.json");
        std::fs::create_dir_all(real.parent().unwrap()).unwrap();
        std::fs::write(&real, r#"{"a":1}"#).unwrap();
        let link = t.path().join("settings.json");
        symlink(&real, &link).unwrap();
        assert_eq!(merge_claude_hooks(&link).unwrap().status, "added");
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(hooks_state(&real), "on");
        assert_eq!(read(&real)["a"], 1);
    }

    #[test]
    fn merge_keeps_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("settings.json");
        std::fs::write(&p, "{}").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        merge_claude_hooks(&p).unwrap();
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn detection_and_status_use_home() {
        let t = tempfile::tempdir().unwrap();
        let h = t.path();
        std::fs::create_dir_all(h.join(".claude")).unwrap();
        std::fs::create_dir_all(h.join(".t3")).unwrap();
        let st = status_in(h);
        let by = |id| st.agents.iter().find(|a| a.id == id).unwrap();
        assert!(by("claude").detected);
        assert!(!by("codex").detected);
        assert!(by("t3").detected);
        assert_eq!(by("claude").skill.as_ref().unwrap().status, "missing");
        assert_eq!(by("claude").hooks, Some("off"));
        assert!(by("t3").skill.is_none());
        assert!(st.render(Style::PLAIN).contains("run: cmw agents install"));
        install_in(h, &AgentsInstallArgs::default()).unwrap();
        let st = status_in(h);
        assert_eq!(st.agents[0].skill.as_ref().unwrap().status, "installed");
        assert!(!st.render(Style::PLAIN).contains("run:"));
        assert!(!h.join(".agents").exists());
    }

    #[test]
    fn install_falls_back_to_agents_dir_when_nothing_detected() {
        let t = tempfile::tempdir().unwrap();
        let r = install_in(t.path(), &AgentsInstallArgs::default()).unwrap();
        assert_eq!(r.skills.len(), 1);
        assert_eq!(r.skills[0].agent, "agents");
        assert!(t.path().join(".agents/skills/cmw/SKILL.md").exists());
    }

    #[test]
    fn explicit_agents_install_even_if_not_detected() {
        let t = tempfile::tempdir().unwrap();
        let args = AgentsInstallArgs {
            agents: vec![AgentId::Grok, AgentId::Grok],
            ..Default::default()
        };
        let r = install_in(t.path(), &args).unwrap();
        assert_eq!(r.skills.len(), 1);
        assert!(t.path().join(".grok/skills/cmw/SKILL.md").exists());
    }

    const SKILL_V013: &str = include_str!("../tests/fixtures/SKILL-v0.1.3.md");

    #[test]
    fn shipped_hashes_include_current_and_old_skills() {
        assert!(
            is_shipped(SKILL_MD),
            "add sha256 {} of assets/agent/SKILL.md to SHIPPED_SKILL_SHA256",
            sha256_hex(SKILL_MD)
        );
        assert!(is_shipped(SKILL_V013));
        assert!(!is_shipped(&format!("{SKILL_V013}\n## My notes\n")));
    }

    fn write(p: &Path, text: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn refresh_only_replaces_unedited_cmw_skills() {
        let t = tempfile::tempdir().unwrap();
        let h = t.path();
        let refresh = AgentsInstallArgs {
            refresh: true,
            ..Default::default()
        };
        assert!(install_in(h, &refresh).unwrap().skills.is_empty());
        assert!(!h.join(".claude").exists());

        // Claude: an old release's skill with the person's own notes added.
        let edited = h.join(".claude/skills/cmw/SKILL.md");
        let edited_text = format!("{SKILL_V013}\n## My notes\nAlways pass --no-slow.\n");
        write(&edited, &edited_text);
        // Codex: exactly what v0.1.3 installed.
        let old = h.join(".agents/skills/cmw/SKILL.md");
        write(&old, SKILL_V013);
        // Grok: current.
        let current = h.join(".grok/skills/cmw/SKILL.md");
        write(&current, SKILL_MD);

        let st = status_in(h);
        let status = |id: &str| {
            st.agents
                .iter()
                .find(|a| a.id == id)
                .unwrap()
                .skill
                .as_ref()
                .unwrap()
                .status
        };
        assert_eq!(status("claude"), "modified");
        assert_eq!(status("codex"), "outdated");
        assert_eq!(status("grok"), "installed");

        let r = install_in(h, &refresh).unwrap();
        let st = |id: &str| {
            r.skills
                .iter()
                .find(|s| s.agent == id)
                .unwrap()
                .status
                .clone()
        };
        assert_eq!(st("claude"), "skipped");
        assert_eq!(st("codex"), "updated");
        assert_eq!(st("grok"), "unchanged");
        assert_eq!(std::fs::read_to_string(&edited).unwrap(), edited_text);
        assert_eq!(std::fs::read_to_string(&old).unwrap(), SKILL_MD);
    }

    #[test]
    fn refresh_honors_agent_filter() {
        let t = tempfile::tempdir().unwrap();
        let h = t.path();
        let claude = h.join(".claude/skills/cmw/SKILL.md");
        let codex = h.join(".agents/skills/cmw/SKILL.md");
        write(&claude, SKILL_V013);
        write(&codex, SKILL_V013);
        let r = install_in(
            h,
            &AgentsInstallArgs {
                refresh: true,
                agents: vec![AgentId::Codex],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(r.skills.len(), 1);
        assert_eq!(r.skills[0].agent, "codex");
        assert_eq!(std::fs::read_to_string(&codex).unwrap(), SKILL_MD);
        assert_eq!(std::fs::read_to_string(&claude).unwrap(), SKILL_V013);
    }

    #[test]
    fn merge_refuses_dangling_symlink() {
        let t = tempfile::tempdir().unwrap();
        let link = t.path().join("settings.json");
        symlink(t.path().join("dotfiles/settings.json"), &link).unwrap();
        assert!(merge_claude_hooks(&link).is_err());
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(!t.path().join("dotfiles").exists());
    }

    #[test]
    fn write_atomic_backs_off_when_file_changed_since_read() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("settings.json");
        std::fs::write(&p, r#"{"theme":"dark"}"#).unwrap();
        // Read `{}`, but another process wrote `theme` before we got to the rename.
        assert!(!write_atomic(&p, "{\"hooks\":{}}\n", Some("{}")).unwrap());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), r#"{"theme":"dark"}"#);
        assert_eq!(
            std::fs::read_dir(t.path()).unwrap().count(),
            1,
            "temp file left behind"
        );
        // A file that appeared after we saw none.
        let q = t.path().join("new.json");
        std::fs::write(&q, "{}").unwrap();
        assert!(!write_atomic(&q, "x", None).unwrap());
        assert!(write_atomic(&p, "{}\n", Some(r#"{"theme":"dark"}"#)).unwrap());
    }

    #[test]
    fn prompt_text_is_the_asset() {
        assert_eq!(prompt(false).unwrap().text.unwrap(), PROMPT_MD);
    }
}
