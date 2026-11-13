mod common;
use common::*;
use predicates::prelude::*;
use std::io::Write;
use std::process::Stdio;

fn stdout(o: &std::process::Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}

fn gone(p: &std::path::Path) -> bool {
    for _ in 0..50 {
        if !p.exists() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    false
}

#[test]
fn new_plain_prints_path_and_hydrates() {
    let sb = Sandbox::new();
    let wt = sb.wt("wt");
    let out = sb.run(&["new", "feat/x", "--path", s(&wt)]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stdout(&out), format!("{}\n", wt.display()));
    assert!(wt.join("node_modules/pkg/index.js").exists());
    assert!(wt.join(".env").exists());
    assert!(!wt.join("build").exists());
    let again = sb.run(&["new", "feat/x", "--path", s(&wt)]);
    assert_eq!(again.status.code(), Some(0));
    assert_eq!(stdout(&again), format!("{}\n", wt.display()));
}

#[test]
fn new_json_envelope() {
    let sb = Sandbox::new();
    let wt = sb.wt("wt");
    let (code, v) = sb.json(&["new", "feat/x", "--path", s(&wt)]);
    assert_eq!(code, 0);
    assert_eq!(v["ok"], true);
    assert_eq!(v["command"], "new");
    assert_eq!(v["data"]["path"], s(&wt));
    assert_eq!(v["data"]["created"], true);
    assert_eq!(v["data"]["hydrate"]["ready"], true);
    assert!(v["warnings"].is_array());
}

#[test]
fn new_json_via_env_var() {
    let sb = Sandbox::new();
    let wt2 = sb.wt("wt2");
    let out = sb
        .cmd()
        .env("CMW_JSON", "1")
        .args(["new", "feat/y", "--path", s(&wt2)])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["command"], "new");
    assert_eq!(v["data"]["created"], true);
    assert_eq!(v["data"]["hydrate"]["ready"], true);
    assert!(v["warnings"].is_array());
}

#[test]
fn usage_error_json() {
    let sb = Sandbox::new();
    let (code, v) = sb.json(&["new"]);
    assert_eq!(code, 2);
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["kind"], "usage");
}

#[test]
fn path_found_and_not_found() {
    let sb = Sandbox::new();
    let wt = sb.new_wt("feat/x", "wt", &[]);
    let out = sb.run(&["path", "feat/x"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stdout(&out), format!("{}\n", wt.display()));
    let out = sb.run(&["path", "nope"]);
    assert_eq!(out.status.code(), Some(4));
    let (code, v) = sb.json(&["path", "nope"]);
    assert_eq!(code, 4);
    assert_eq!(v["error"]["kind"], "not_found");
}

#[test]
fn ls_lists_both() {
    let sb = Sandbox::new();
    let wt = sb.new_wt("feat/x", "wt", &[]);
    let (code, v) = sb.json(&["ls"]);
    assert_eq!(code, 0);
    let wts = v["data"]["repos"][0]["worktrees"].as_array().unwrap();
    assert_eq!(wts.len(), 2);
    let main = wts
        .iter()
        .find(|w| w["path"] == s(&sb.main))
        .expect("main listed");
    assert_eq!(main["is_main"], true);
    let new = wts.iter().find(|w| w["path"] == s(&wt)).expect("wt listed");
    assert_eq!(new["is_main"], false);
    assert_eq!(new["hydrated"], true);
}

#[test]
fn rm_refuses_dirty() {
    let sb = Sandbox::new();
    let wt = sb.new_wt("feat/x", "wt", &[]);
    write(&wt.join("README.md"), "changed\n");
    let out = sb.run(&["rm", "feat/x"]);
    assert_eq!(out.status.code(), Some(3));
    let (code, v) = sb.json(&["rm", "feat/x"]);
    assert_eq!(code, 3);
    assert_eq!(v["error"]["kind"], "refused");
    assert!(v["error"]["hint"].as_str().unwrap().contains("--force"));
    assert!(wt.exists());
}

#[test]
fn rm_refuses_unmerged_with_remote() {
    let sb = Sandbox::new();
    let bare = sb.root.join("origin.git");
    std::fs::create_dir_all(&bare).unwrap();
    git(&bare, &["init", "-q", "--bare", "-b", "main"]);
    git(&sb.main, &["remote", "add", "origin", s(&bare)]);
    git(&sb.main, &["push", "-q", "origin", "main"]);
    let wt = sb.new_wt("feat/x", "wt", &[]);
    sb.commit(&wt, "f.txt", "f\n", "unpushed work");
    let out = sb.run(&["rm", "feat/x"]);
    assert_eq!(
        out.status.code(),
        Some(3),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(wt.exists());
}

#[test]
fn rm_happy_path_merged() {
    let sb = Sandbox::new();
    let wt = sb.new_wt("feat/x", "wt", &[]);
    sb.merge_branch(&wt, "feat/x");
    let out = sb.run(&["rm", "feat/x"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(gone(&wt));
    let branches = git(&sb.main, &["branch", "--list", "feat/x"]);
    assert!(
        branches.trim().is_empty(),
        "branch still present: {branches}"
    );
    let list = git(&sb.main, &["worktree", "list"]);
    assert!(!list.contains(s(&wt)), "{list}");
}

#[test]
fn gc_dry_run_and_yes() {
    let sb = Sandbox::new();
    let merged = sb.new_wt("feat/merged", "wt-merged", &[]);
    sb.merge_branch(&merged, "feat/merged");
    let fresh = sb.new_wt("feat/fresh", "wt-fresh", &[]);

    let (code, v) = sb.json(&["gc"]);
    assert_eq!(code, 0);
    assert_eq!(v["data"]["dry_run"], true);
    let would = v["data"]["would_remove"].as_array().unwrap();
    assert!(would.iter().any(|i| i["path"] == s(&merged)), "{v}");
    assert!(!would.iter().any(|i| i["path"] == s(&fresh)), "{v}");
    assert!(merged.exists());

    let (code, v) = sb.json(&["gc", "--yes"]);
    assert_eq!(code, 0);
    assert!(v["data"]["freed_bytes"].is_number(), "{v}");
    assert!(
        v["data"]["removed"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["path"] == s(&merged))
    );
    assert!(gone(&merged));
    assert!(fresh.exists());
}

#[test]
fn rm_main_refused() {
    let sb = Sandbox::new();
    let out = sb.run(&["rm", s(&sb.main)]);
    assert_eq!(out.status.code(), Some(3));
}

#[test]
fn env_sync_copied_kept_force() {
    let sb = Sandbox::new();
    let wt = sb.new_wt("feat/x", "wt", &[]);
    let status = |v: &serde_json::Value| -> String {
        v["data"]["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["path"] == ".env")
            .unwrap()["status"]
            .as_str()
            .unwrap()
            .to_string()
    };
    // (a) missing -> copied
    std::fs::remove_file(wt.join(".env")).unwrap();
    let (code, v) = sb.json(&["env", "sync", s(&wt)]);
    assert_eq!(code, 0);
    assert_eq!(status(&v), "copied");
    assert_eq!(
        std::fs::read_to_string(wt.join(".env")).unwrap(),
        "SECRET=1\n"
    );
    // (b) both differ -> kept + warning
    write(&sb.main.join(".env"), "SECRET=main\n");
    write(&wt.join(".env"), "SECRET=wt\n");
    let (_, v) = sb.json(&["env", "sync", s(&wt)]);
    assert_eq!(status(&v), "kept");
    assert!(!v["warnings"].as_array().unwrap().is_empty());
    assert_eq!(
        std::fs::read_to_string(wt.join(".env")).unwrap(),
        "SECRET=wt\n"
    );
    let (_, v) = sb.json(&["env", "sync", s(&wt), "--force"]);
    assert_eq!(status(&v), "overwritten");
    assert_eq!(
        std::fs::read_to_string(wt.join(".env")).unwrap(),
        "SECRET=main\n"
    );
}

#[test]
fn clean_artifacts_dry_and_yes() {
    let sb = Sandbox::new();
    let wt = sb.new_wt("feat/x", "wt", &[]);
    write(&wt.join("build/out.txt"), "artifact\n");
    let (code, v) = sb.json(&["clean-artifacts", "--idle", "0"]);
    assert_eq!(code, 0);
    assert_eq!(v["data"]["dry_run"], true);
    let listed = v["data"]["worktrees"].to_string();
    assert!(listed.contains("build"), "{v}");
    assert!(wt.join("build").exists());

    let (code, _) = sb.json(&["clean-artifacts", "--idle", "0", "--yes"]);
    assert_eq!(code, 0);
    assert!(gone(&wt.join("build")));
    assert!(wt.join("README.md").exists());
    assert!(wt.join(".gitignore").exists());
    assert!(wt.join("node_modules/pkg/index.js").exists());
}

#[test]
fn hydrate_dry_run_writes_nothing() {
    let sb = Sandbox::new();
    let wt = sb.new_wt("feat/x", "wt", &["--no-hydrate"]);
    assert!(!wt.join(".env").exists());
    let (code, v) = sb.json(&["hydrate", "--dry-run", s(&wt)]);
    assert_eq!(code, 0);
    let entries = v["data"]["entries"].as_array().unwrap();
    let action = |p: &str| {
        entries
            .iter()
            .find(|e| e["path"] == p)
            .map(|e| e["action"].as_str().unwrap().to_string())
    };
    assert_eq!(
        action("node_modules/").as_deref(),
        Some("would_clone"),
        "{v}"
    );
    assert_eq!(action(".env").as_deref(), Some("would_copy"), "{v}");
    assert!(!wt.join(".env").exists());
    assert!(!wt.join("node_modules").exists());
}

#[test]
fn shell_init_zsh() {
    let sb = Sandbox::new();
    sb.cmd()
        .args(["shell-init", "zsh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("command cmw path"));
}

#[test]
fn skill_print_and_install() {
    let sb = Sandbox::new();
    sb.cmd()
        .args(["skill", "print"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--json"));
    let dir = sb.root.join("skills/cmw");
    let (code, v) = sb.json(&["skill", "install", "--dir", s(&dir)]);
    assert_eq!(code, 0);
    assert_eq!(v["data"]["status"], "created");
    assert!(dir.join("SKILL.md").exists());
    let (_, v) = sb.json(&["skill", "install", "--dir", s(&dir)]);
    assert_eq!(v["data"]["status"], "unchanged");
}

#[test]
fn hook_claude_create() {
    let sb = Sandbox::new();
    let payload = format!(
        r#"{{"session_id":"s","transcript_path":"/tmp/t","cwd":"{}","hook_event_name":"WorktreeCreate","name":"hook-wt"}}"#,
        sb.main.display()
    );
    let mut child = sb
        .std_cmd(&["hook", "claude-create"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = stdout(&out);
    let last = text.lines().rev().find(|l| !l.trim().is_empty()).unwrap();
    let p = std::path::Path::new(last);
    assert!(p.is_absolute(), "{last}");
    assert!(p.exists(), "{last}");
    assert!(p.starts_with(sb.cmw_home.join("worktrees")), "{last}");
}

#[test]
fn concurrent_new() {
    let sb = Sandbox::new();
    let (a, b) = (sb.wt("wa"), sb.wt("wb"));
    let c1 = sb
        .std_cmd(&["new", "feat/a", "--path", s(&a)])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let c2 = sb
        .std_cmd(&["new", "feat/b", "--path", s(&b)])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    for c in [c1, c2] {
        let o = c.wait_with_output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }
    assert!(a.join(".git").exists() && b.join(".git").exists());
    let list = git(&sb.main, &["worktree", "list"]);
    assert!(list.contains(s(&a)) && list.contains(s(&b)), "{list}");
}

#[test]
fn shell_init_stays_raw_with_cmw_json() {
    let sb = common::Sandbox::new();
    let out = sb
        .cmd()
        .env("CMW_JSON", "1")
        .args(["shell-init", "zsh"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        text.contains("command cmw path"),
        "expected raw script, got: {text}"
    );
    assert!(!text.trim_start().starts_with('{'));
}

#[test]
fn rm_refuses_worktree_with_missing_git_file() {
    let sb = Sandbox::new();
    let wt = sb.new_wt("feat/broken", "brk", &[]);
    write(&wt.join("precious"), "keep me\n");
    std::fs::remove_file(wt.join(".git")).unwrap();
    let out = sb.run(&["rm", "feat/broken"]);
    assert_eq!(
        out.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("--force"));
    assert!(wt.join("precious").exists());
}

#[test]
fn bare_cmw_without_a_terminal_prints_help_and_exits_2() {
    // Agents and pipes never get the TUI: the pre-TUI contract (help on stderr, exit 2) holds.
    let sb = Sandbox::new();
    let out = sb.run(&[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("Usage: cmw"), "{err}");
}

#[test]
fn bare_cmw_json_is_a_usage_error() {
    let sb = Sandbox::new();
    let (code, v) = sb.json(&[]);
    assert_eq!(code, 2);
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["kind"], "usage");
}

/// Serve one canned HTTP response per connection, like GitHub's `releases/latest` redirect.
fn fake_releases(status_line: &'static str, location: &'static str) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let _ = write!(
                stream,
                "HTTP/1.1 {status_line}\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
        }
    });
    format!("http://{addr}/EQuimper/cleanmyworktree/releases/latest")
}

#[test]
fn update_check_reports_a_newer_release() {
    let sb = Sandbox::new();
    let url = fake_releases(
        "302 Found",
        "https://github.com/EQuimper/cleanmyworktree/releases/tag/v99.0.0",
    );
    let out = sb
        .cmd()
        .args(["--json", "update", "--check"])
        .env("CMW_UPDATE_URL", &url)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["latest"], "99.0.0");
    assert_eq!(v["data"]["update_available"], true);
    assert_eq!(v["data"]["updated"], false);
}

#[test]
fn update_refuses_for_a_source_build_with_a_fix() {
    // Test binaries live in target/, i.e. not installed by Homebrew, the installer or cargo.
    let sb = Sandbox::new();
    let url = fake_releases(
        "302 Found",
        "https://github.com/EQuimper/cleanmyworktree/releases/tag/v99.0.0",
    );
    let out = sb
        .cmd()
        .args(["--json", "update"])
        .env("CMW_UPDATE_URL", &url)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["error"]["kind"], "refused");
    assert!(
        v["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("brew install")
    );
}

#[test]
fn update_with_no_release_is_up_to_date_and_offline_failure_is_an_error() {
    let sb = Sandbox::new();
    let url = fake_releases("404 Not Found", "");
    let out = sb
        .cmd()
        .args(["update"])
        .env("CMW_UPDATE_URL", &url)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("no published release"));

    let (code, v) = {
        let out = sb
            .cmd()
            .args(["--json", "update"])
            .env("CMW_UPDATE_URL", "http://127.0.0.1:9/")
            .output()
            .unwrap();
        (
            out.status.code().unwrap(),
            serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap(),
        )
    };
    assert_eq!(code, 1);
    assert_eq!(v["ok"], false);
}

fn fake_agents(sb: &Sandbox) {
    std::fs::create_dir_all(sb.home.join(".claude")).unwrap();
    std::fs::create_dir_all(sb.home.join(".codex")).unwrap();
}

#[test]
fn agents_status_json() {
    let sb = Sandbox::new();
    fake_agents(&sb);
    let (code, v) = sb.json(&["agents"]);
    assert_eq!(code, 0);
    assert_eq!(v["command"], "agents");
    let agents = v["data"]["agents"].as_array().unwrap();
    let by = |id: &str| agents.iter().find(|a| a["id"] == id).unwrap();
    assert_eq!(by("claude")["detected"], true);
    assert_eq!(by("claude")["skill"]["status"], "missing");
    assert_eq!(by("claude")["hooks"], "off");
    assert_eq!(by("codex")["detected"], true);
    assert!(by("codex").get("hooks").is_none());
    assert!(
        by("codex")["skill"]["path"]
            .as_str()
            .unwrap()
            .ends_with(".agents/skills/cmw/SKILL.md")
    );
    assert_eq!(by("grok")["detected"], false);
    assert_eq!(by("t3")["detected"], false);
    assert_eq!(by("t3")["worktree_setup"], "cmw hydrate");
    assert!(by("t3").get("skill").is_none());
    assert_eq!(v["data"]["prompt"], "cmw agents prompt --copy");
    // Envelope order is ok, command, data, warnings.
    let keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys, ["ok", "command", "data", "warnings"]);
}

#[test]
fn agents_install_creates_then_unchanged() {
    let sb = Sandbox::new();
    fake_agents(&sb);
    let (code, v) = sb.json(&["agents", "install"]);
    assert_eq!(code, 0);
    let skills = v["data"]["skills"].as_array().unwrap();
    assert_eq!(skills.len(), 2);
    assert!(skills.iter().all(|s| s["status"] == "created"));
    assert!(v["data"]["claude_hooks"].is_null());
    assert!(sb.home.join(".claude/skills/cmw/SKILL.md").exists());
    assert!(sb.home.join(".agents/skills/cmw/SKILL.md").exists());
    assert!(!sb.home.join(".grok").exists());
    let (_, v) = sb.json(&["agents", "install"]);
    assert!(
        v["data"]["skills"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["status"] == "unchanged")
    );
    let (_, v) = sb.json(&["agents"]);
    assert_eq!(v["data"]["agents"][0]["skill"]["status"], "installed");
}

#[test]
fn agents_install_refresh_and_hooks_flag() {
    let sb = Sandbox::new();
    fake_agents(&sb);
    let (_, v) = sb.json(&["agents", "install", "--refresh"]);
    assert_eq!(v["data"]["skills"].as_array().unwrap().len(), 0);
    assert!(!sb.home.join(".claude/skills").exists());
    let (_, v) = sb.json(&["agents", "install", "--agent", "grok", "--claude-hooks"]);
    assert_eq!(v["data"]["skills"][0]["agent"], "grok");
    assert_eq!(v["data"]["claude_hooks"]["status"], "added");
    let v013 = include_str!("fixtures/SKILL-v0.1.3.md");
    write(&sb.home.join(".grok/skills/cmw/SKILL.md"), v013);
    let (_, v) = sb.json(&["agents", "install", "--refresh"]);
    assert_eq!(v["data"]["skills"][0]["status"], "updated");
    // An edited skill survives `--refresh` (which `cmw update` runs).
    let edited = format!("{v013}\n## Mine\n");
    write(&sb.home.join(".grok/skills/cmw/SKILL.md"), &edited);
    let (_, v) = sb.json(&["agents", "install", "--refresh"]);
    assert_eq!(v["data"]["skills"][0]["status"], "skipped");
    assert_eq!(
        std::fs::read_to_string(sb.home.join(".grok/skills/cmw/SKILL.md")).unwrap(),
        edited
    );
}

#[test]
fn agents_prompt_matches_asset() {
    let sb = Sandbox::new();
    let (code, v) = sb.json(&["agents", "prompt"]);
    assert_eq!(code, 0);
    let asset = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/agent/prompt.md"
    ))
    .unwrap();
    assert_eq!(v["data"]["text"], asset.as_str());
}

#[test]
fn hook_claude_install_is_idempotent() {
    let sb = Sandbox::new();
    let (code, v) = sb.json(&["hook", "claude", "--install"]);
    assert_eq!(code, 0);
    assert_eq!(v["data"]["status"], "added");
    let settings = sb.home.join(".claude/settings.json");
    assert_eq!(v["data"]["path"], s(&settings));
    let (_, v) = sb.json(&["hook", "claude", "--install"]);
    assert_eq!(v["data"]["status"], "unchanged");
    let (_, v) = sb.json(&["agents"]);
    assert_eq!(v["data"]["agents"][0]["hooks"], "on");
}

#[test]
fn hook_claude_install_project_and_invalid_settings() {
    let sb = Sandbox::new();
    let (code, v) = sb.json(&["hook", "claude", "--install", "--project"]);
    assert_eq!(code, 0);
    assert_eq!(v["data"]["path"], s(&sb.main.join(".claude/settings.json")));
    write(&sb.home.join(".claude/settings.json"), "{oops");
    let (code, v) = sb.json(&["hook", "claude", "--install"]);
    assert_eq!(code, 1);
    assert_eq!(v["ok"], false);
    assert!(v["error"]["hint"].is_string());
    assert_eq!(
        std::fs::read_to_string(sb.home.join(".claude/settings.json")).unwrap(),
        "{oops"
    );
}

#[test]
fn skill_install_without_dir_installs_for_agents() {
    let sb = Sandbox::new();
    fake_agents(&sb);
    let (code, v) = sb.json(&["skill", "install"]);
    assert_eq!(code, 0);
    assert_eq!(v["data"]["skills"].as_array().unwrap().len(), 2);
}

/// The landing page playground prints this help text and output. After changing src/cli.rs,
/// run `cargo build --release && python3 website/scripts/cli-snapshot.py`.
#[test]
fn website_playground_matches_cli() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("website/src/lib/cli-snapshot.json");
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let run = |args: Vec<&str>| {
        let out = std::process::Command::new(assert_cmd::cargo::cargo_bin("cmw"))
            .args(args)
            .env_remove("CMW_JSON")
            .output()
            .unwrap();
        stdout(&out).trim_end().to_string()
    };
    for (cmd, entry) in saved["help"].as_object().unwrap() {
        let args = |flag| cmd.split_whitespace().chain([flag]).collect::<Vec<_>>();
        let long = entry["long"].as_str().unwrap();
        let short = entry["short"].as_str().unwrap_or(long);
        assert_eq!(
            run(args("--help")),
            long,
            "`cmw {cmd} --help` changed; regenerate cli-snapshot.json"
        );
        assert_eq!(
            run(args("-h")),
            short,
            "`cmw {cmd} -h` changed; regenerate cli-snapshot.json"
        );
    }
    for (cmd, text) in saved["output"].as_object().unwrap() {
        let args = cmd.split_whitespace().collect();
        assert_eq!(
            run(args),
            text.as_str().unwrap(),
            "`cmw {cmd}` changed; regenerate cli-snapshot.json"
        );
    }
}
