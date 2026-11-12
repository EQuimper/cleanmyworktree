//! `cmw ls` / `cmw path` and per-worktree status. STUB: owned by worker B.
use crate::cli::{LsArgs, PathArgs};
use crate::output::{Ctx, Report, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub struct LsReport {}
impl Report for LsReport {
    fn human(&self) -> String {
        String::new()
    }
}

#[derive(Debug, Serialize)]
pub struct PathReport {
    pub path: PathBuf,
    pub branch: Option<String>,
}
impl Report for PathReport {
    fn human(&self) -> String {
        self.path.display().to_string()
    }
}

pub fn ls(_ctx: &Ctx, _cwd: &Path, _args: &LsArgs) -> Result<LsReport> {
    todo!("worker B")
}

pub fn path(_ctx: &Ctx, cwd: &Path, args: &PathArgs) -> Result<PathReport> {
    let worktree = crate::git::resolve_worktree(cwd, &args.name)?;
    Ok(PathReport { path: worktree.path, branch: worktree.branch })
}
