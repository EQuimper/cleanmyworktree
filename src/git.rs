//! Thin wrapper over the `git` CLI. We shell out instead of using gix because worktree
//! add/remove/prune support there is incomplete, and git is always installed.

use crate::output::{CmwError, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Run git in `cwd`, returning trimmed stdout. Fails with git's stderr as the message.
pub fn run(cwd: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| CmwError::error(format!("failed to run git: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(CmwError::error(format!("git {} failed: {stderr}", args.join(" "))));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim_end_matches(['\n', '\r']).to_string())
}

/// Run git, returning raw stdout bytes (for `-z` output).
pub fn run_raw(cwd: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| CmwError::error(format!("failed to run git: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(CmwError::error(format!("git {} failed: {stderr}", args.join(" "))));
    }
    Ok(out.stdout)
}

/// True when the git command exits 0. Output is discarded.
pub fn run_ok(cwd: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Top level of the worktree containing `cwd`.
pub fn toplevel(cwd: &Path) -> Result<PathBuf> {
    run(cwd, &["rev-parse", "--show-toplevel"])
        .map(PathBuf::from)
        .map_err(|_| CmwError::not_found(format!("not inside a git worktree: {}", cwd.display())))
}

/// Shared `.git` directory for all worktrees of the repo (absolute).
pub fn common_dir(cwd: &Path) -> Result<PathBuf> {
    run(cwd, &["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .map(PathBuf::from)
        .map_err(|_| CmwError::not_found(format!("not inside a git repository: {}", cwd.display())))
}

/// Per-worktree git directory (absolute), e.g. `.git/worktrees/<name>`. cmw keeps its
/// per-worktree state here so it never shows up as an untracked file.
pub fn git_dir(cwd: &Path) -> Result<PathBuf> {
    run(cwd, &["rev-parse", "--path-format=absolute", "--git-dir"]).map(PathBuf::from)
}

#[derive(Debug, Clone, Serialize)]
pub struct Worktree {
    pub path: PathBuf,
    pub head: Option<String>,
    /// Short branch name (`feat/x`), None when detached.
    pub branch: Option<String>,
    pub bare: bool,
    pub detached: bool,
    pub locked: bool,
    /// Git reports the directory as missing; `git worktree prune` will drop it.
    pub prunable: bool,
    /// The repository's main checkout (first entry of `git worktree list`).
    pub is_main: bool,
}

/// All worktrees of the repository that contains `cwd`. First entry is the main checkout.
pub fn list_worktrees(cwd: &Path) -> Result<Vec<Worktree>> {
    let raw = run_raw(cwd, &["worktree", "list", "--porcelain", "-z"])?;
    let text = String::from_utf8_lossy(&raw);
    let mut out = Vec::new();
    let mut cur: Option<Worktree> = None;
    for field in text.split('\0') {
        if field.is_empty() {
            if let Some(wt) = cur.take() {
                out.push(wt);
            }
            continue;
        }
        let (key, val) = field.split_once(' ').unwrap_or((field, ""));
        match key {
            "worktree" => {
                if let Some(wt) = cur.take() {
                    out.push(wt);
                }
                cur = Some(Worktree {
                    path: PathBuf::from(val),
                    head: None,
                    branch: None,
                    bare: false,
                    detached: false,
                    locked: false,
                    prunable: false,
                    is_main: false,
                });
            }
            "HEAD" => {
                if let Some(wt) = cur.as_mut() {
                    wt.head = Some(val.to_string());
                }
            }
            "branch" => {
                if let Some(wt) = cur.as_mut() {
                    wt.branch = Some(val.strip_prefix("refs/heads/").unwrap_or(val).to_string());
                }
            }
            "bare" => cur.iter_mut().for_each(|w| w.bare = true),
            "detached" => cur.iter_mut().for_each(|w| w.detached = true),
            "locked" => cur.iter_mut().for_each(|w| w.locked = true),
            "prunable" => cur.iter_mut().for_each(|w| w.prunable = true),
            _ => {}
        }
    }
    if let Some(wt) = cur.take() {
        out.push(wt);
    }
    if let Some(first) = out.first_mut() {
        first.is_main = true;
    }
    Ok(out)
}

/// The main checkout, used as the hydrate source. None for bare repositories.
pub fn main_worktree(cwd: &Path) -> Result<Option<Worktree>> {
    Ok(list_worktrees(cwd)?.into_iter().find(|w| w.is_main && !w.bare))
}

/// Find a worktree by path, branch name, branch slug, or directory name.
pub fn resolve_worktree(cwd: &Path, query: &str) -> Result<Worktree> {
    let wts = list_worktrees(cwd)?;
    let as_path = Path::new(query);
    let canon = std::fs::canonicalize(if as_path.is_absolute() { as_path.to_path_buf() } else { cwd.join(as_path) }).ok();
    for wt in &wts {
        let wt_canon = std::fs::canonicalize(&wt.path).unwrap_or_else(|_| wt.path.clone());
        if canon.as_ref() == Some(&wt_canon) || wt.path == as_path {
            return Ok(wt.clone());
        }
    }
    let matches: Vec<&Worktree> = wts
        .iter()
        .filter(|wt| {
            wt.branch.as_deref() == Some(query)
                || wt.branch.as_deref().map(slugify).as_deref() == Some(query)
                || wt.path.file_name().and_then(|n| n.to_str()) == Some(query)
        })
        .collect();
    match matches.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(CmwError::not_found(format!("no worktree matches '{query}'")).with_hint("cmw ls")),
        many => Err(CmwError::usage(format!(
            "'{query}' matches {} worktrees; pass a path instead: {}",
            many.len(),
            many.iter().map(|w| w.path.display().to_string()).collect::<Vec<_>>().join(", ")
        ))),
    }
}

/// Default branch name (e.g. `main`), from origin/HEAD, else main/master, else the main checkout's branch.
pub fn default_branch(cwd: &Path) -> Result<String> {
    if let Ok(r) = run(cwd, &["symbolic-ref", "--quiet", "--short", "refs/remotes/origin/HEAD"])
        && let Some(b) = r.strip_prefix("origin/")
    {
        return Ok(b.to_string());
    }
    for b in ["main", "master"] {
        if run_ok(cwd, &["show-ref", "--verify", "--quiet", &format!("refs/heads/{b}")]) {
            return Ok(b.to_string());
        }
    }
    main_worktree(cwd)?
        .and_then(|w| w.branch)
        .ok_or_else(|| CmwError::error("could not determine the default branch"))
}

/// Ignored, untracked entries under `root`, relative to it. Fully ignored directories are
/// reported once with a trailing `/` (git's `--directory` collapsing), files without.
pub fn ignored_entries(root: &Path) -> Result<Vec<String>> {
    let raw = run_raw(root, &["ls-files", "--others", "--ignored", "--exclude-standard", "--directory", "-z"])?;
    Ok(String::from_utf8_lossy(&raw)
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect())
}

/// Number of changed or untracked (non-ignored) paths. 0 means clean.
pub fn dirty_count(worktree: &Path) -> Result<usize> {
    let raw = run_raw(worktree, &["status", "--porcelain=v1", "-z", "--untracked-files=normal"])?;
    // Renames emit two NUL-separated paths; count only entries that start with a status code.
    let text = String::from_utf8_lossy(&raw);
    let mut count = 0;
    let mut skip_next = false;
    for entry in text.split('\0').filter(|s| !s.is_empty()) {
        if skip_next {
            skip_next = false;
            continue;
        }
        if entry.starts_with('R') || entry.starts_with('C') {
            skip_next = true;
        }
        count += 1;
    }
    Ok(count)
}

/// Filesystem-safe name for a branch: `feat/Login UI` -> `feat-login-ui`.
pub fn slugify(branch: &str) -> String {
    let mut s = String::with_capacity(branch.len());
    let mut last_dash = false;
    for c in branch.chars() {
        if c.is_ascii_alphanumeric() || c == '.' || c == '_' {
            s.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            s.push('-');
            last_dash = true;
        }
    }
    s.trim_matches('-').to_string()
}

/// Repository name derived from the common dir: `/x/foo/.git` -> `foo`, `/x/foo.git` -> `foo`.
pub fn repo_name(common_dir: &Path) -> String {
    let base = if common_dir.file_name().and_then(|n| n.to_str()) == Some(".git") {
        common_dir.parent().unwrap_or(common_dir)
    } else {
        common_dir
    };
    let name = base.file_name().and_then(|n| n.to_str()).unwrap_or("repo");
    name.strip_suffix(".git").unwrap_or(name).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_branches() {
        assert_eq!(slugify("feat/Login UI"), "feat-login-ui");
        assert_eq!(slugify("--x--"), "x");
        assert_eq!(slugify("v1.2_fix"), "v1.2_fix");
    }

    #[test]
    fn repo_names() {
        assert_eq!(repo_name(Path::new("/a/foo/.git")), "foo");
        assert_eq!(repo_name(Path::new("/a/foo.git")), "foo");
    }
}
