use std::path::PathBuf;

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// `$CMW_HOME`, default `~/.cmw`.
pub fn cmw_home() -> PathBuf {
    std::env::var_os("CMW_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".cmw"))
}

/// Where `cmw new` puts worktrees: `<cmw_home>/worktrees/<repo>/<branch-slug>`.
pub fn worktrees_root() -> PathBuf {
    cmw_home().join("worktrees")
}

/// Directories scanned by `--all`: cmw's own root, T3 Code's worktrees, and `$CMW_SCAN_ROOTS`
/// (colon-separated). Only existing directories are returned, deduplicated.
pub fn scan_roots() -> Vec<PathBuf> {
    let mut roots = vec![worktrees_root(), home().join(".t3/worktrees")];
    if let Some(extra) = std::env::var_os("CMW_SCAN_ROOTS") {
        roots.extend(std::env::split_paths(&extra));
    }
    let mut out: Vec<PathBuf> = Vec::new();
    for r in roots {
        if r.is_dir() && !out.contains(&r) {
            out.push(r);
        }
    }
    out
}
