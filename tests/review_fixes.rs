//! Regression tests for code-review findings fixed in main.rs / new.rs / skill.rs.
mod common;
use common::{Sandbox, git, write};
use std::os::unix::io::AsRawFd;

/// Finding: the lock was taken on cwd's repo even when `hydrate <path>` targets another repo.
#[test]
fn hydrate_locks_the_target_repo_not_cwd() {
    let a = Sandbox::new();
    let b = Sandbox::new();
    let b_wt = b.new_wt("feat/b", "bwt", &["--no-hydrate"]);

    // Hold repo B's lock from this test process.
    let common = git(&b.main, &["rev-parse", "--path-format=absolute", "--git-common-dir"]);
    let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(std::path::Path::new(common.trim()).join("cmw.lock")).unwrap();
    assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) }, 0);

    let out = a
        .cmd()
        .env("CMW_LOCK_TIMEOUT_SECS", "1")
        .args(["--json", "hydrate", b_wt.to_str().unwrap(), "--no-install"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "expected lock timeout, got {}", String::from_utf8_lossy(&out.stdout));
    assert!(!b_wt.join(".env").exists(), "hydrate wrote into a repo whose lock was held");
    drop(lock);
}

/// Finding: `new` turned a hydrate error into exit 0 with `hydrate: null`.
#[test]
fn new_fails_loudly_when_hydrate_fails() {
    let sb = Sandbox::new();
    write(&sb.main.join(".cmw.toml"), "not_a_key = 1\n");
    git(&sb.main, &["add", ".cmw.toml"]);
    git(&sb.main, &["commit", "-q", "-m", "bad config"]);

    let p = sb.wt("bad");
    let (code, v) = sb.json(&["new", "feat/bad", "--path", p.to_str().unwrap()]);
    assert_eq!(code, 2, "invalid .cmw.toml is a usage error: {v}");
    assert_eq!(v["ok"], false);
    let msg = v["error"]["message"].as_str().unwrap();
    assert!(msg.contains("worktree created at") && msg.contains("hydrate failed"), "{msg}");
    assert!(v["error"]["hint"].as_str().is_some());
    assert!(p.exists(), "worktree itself should still exist for a retry");
}

/// Finding: inherited CMW_JSON=1 wrapped the WorktreeCreate hook's path in a JSON envelope.
#[test]
fn hook_create_prints_bare_path_even_with_cmw_json() {
    let sb = Sandbox::new();
    let payload = format!(
        r#"{{"session_id":"s","transcript_path":"/tmp/t","cwd":"{}","hook_event_name":"WorktreeCreate","name":"hook-json"}}"#,
        sb.main.display()
    );
    let out = sb.cmd().env("CMW_JSON", "1").args(["hook", "claude-create"]).write_stdin(payload).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    let last = stdout.lines().rev().find(|l| !l.trim().is_empty()).unwrap();
    assert!(last.starts_with('/') && std::path::Path::new(last).is_dir(), "last line must be a path: {stdout}");
}

/// Finding: with `-C other` and a relative target, the lock resolved against `other` while hydrate
/// resolved against the process cwd, so the mutated repo was not locked.
#[test]
fn relative_target_with_dash_c_resolves_once() {
    let start = Sandbox::new();
    let other = Sandbox::new();
    // `target` exists in both repos' parent dirs: start.root/target (worktree of start) and other.root/target.
    let start_wt = start.new_wt("feat/s", "target", &["--no-hydrate"]);
    let other_wt = other.new_wt("feat/o", "target", &["--no-hydrate"]);

    // From start.root, `-C <other.root> hydrate target` must hydrate other.root/target (relative to -C), not start's.
    let out = start
        .cmd()
        .current_dir(&start.root)
        .args(["-C", other.root.to_str().unwrap(), "--json", "hydrate", "target", "--no-install"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));
    assert!(other_wt.join(".env").exists(), "-C relative target should be hydrated");
    assert!(!start_wt.join(".env").exists(), "process-cwd target must not be touched");
}
