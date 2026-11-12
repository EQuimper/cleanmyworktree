#![allow(dead_code)]
use assert_cmd::Command;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Output;
use tempfile::TempDir;

/// Isolated sandbox: HOME, CMW_HOME, CMW_SCAN_ROOTS and a main repo, all under one tempdir.
pub struct Sandbox {
    pub tmp: TempDir,
    pub root: PathBuf,
    pub home: PathBuf,
    pub cmw_home: PathBuf,
    pub scan: PathBuf,
    pub main: PathBuf,
}

pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {args:?} in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

pub fn write(path: &Path, content: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

pub fn init_repo(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.name", "Test"]);
    git(dir, &["config", "user.email", "test@example.com"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
    write(&dir.join("README.md"), "hello\n");
    write(
        &dir.join(".gitignore"),
        "node_modules/\n.env\nbuild/\nios/\n",
    );
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "init"]);
    write(
        &dir.join("node_modules/pkg/index.js"),
        "module.exports = 1;\n",
    );
    write(&dir.join(".env"), "SECRET=1\n");
    write(&dir.join("build/out.txt"), "out\n");
}

impl Sandbox {
    pub fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let home = root.join("home");
        let cmw_home = root.join("cmwhome");
        let scan = root.join("scan");
        for d in [&home, &cmw_home, &scan] {
            std::fs::create_dir_all(d).unwrap();
        }
        let main = root.join("main");
        init_repo(&main);
        Sandbox {
            tmp,
            root,
            home,
            cmw_home,
            scan,
            main,
        }
    }

    pub fn wt(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// Command with full isolation, run from the main repo.
    pub fn cmd(&self) -> Command {
        let mut c = Command::cargo_bin("cmw").unwrap();
        self.isolate(&mut c);
        c.current_dir(&self.main);
        c
    }

    pub fn isolate(&self, c: &mut Command) {
        c.env("HOME", &self.home)
            .env("CMW_HOME", &self.cmw_home)
            .env("CMW_SCAN_ROOTS", &self.scan)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("CMW_JSON");
    }

    pub fn std_cmd(&self, args: &[&str]) -> std::process::Command {
        let mut c = std::process::Command::new(assert_cmd::cargo::cargo_bin("cmw"));
        c.args(args)
            .current_dir(&self.main)
            .env("HOME", &self.home)
            .env("CMW_HOME", &self.cmw_home)
            .env("CMW_SCAN_ROOTS", &self.scan)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("CMW_JSON");
        c
    }

    pub fn run(&self, args: &[&str]) -> Output {
        self.cmd().args(args).output().unwrap()
    }

    /// Run with --json, parse stdout (any exit code).
    pub fn json(&self, args: &[&str]) -> (i32, Value) {
        let mut a = vec!["--json"];
        a.extend_from_slice(args);
        let out = self.run(&a);
        let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
            panic!(
                "stdout not JSON ({e}): {}",
                String::from_utf8_lossy(&out.stdout)
            )
        });
        (out.status.code().unwrap(), v)
    }

    /// `cmw new <branch> --path <root>/<dir>` and return the path.
    pub fn new_wt(&self, branch: &str, dir: &str, extra: &[&str]) -> PathBuf {
        let p = self.wt(dir);
        let mut a = vec!["new", branch, "--path", p.to_str().unwrap()];
        a.extend_from_slice(extra);
        let out = self.run(&a);
        assert!(
            out.status.success(),
            "cmw new failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        p
    }

    /// Commit a file in `dir` on its current branch.
    pub fn commit(&self, dir: &Path, file: &str, content: &str, msg: &str) {
        write(&dir.join(file), content);
        git(dir, &["add", file]);
        git(dir, &["commit", "-q", "-m", msg]);
    }

    /// Commit on feat branch in worktree, then merge into main with --no-ff.
    pub fn merge_branch(&self, wt: &Path, branch: &str) {
        self.commit(wt, "feature.txt", "feature\n", "feat work");
        git(
            &self.main,
            &["merge", "--no-ff", "-q", "-m", "merge", branch],
        );
    }
}

pub fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}
