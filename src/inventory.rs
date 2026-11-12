//! `cmw ls` / `cmw path` and per-worktree status. STUB: owned by worker B.
use crate::cli::{LsArgs, PathArgs};
use crate::output::{Ctx, Report, Result};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize)]
pub struct LsReport {}
impl Report for LsReport {
    fn human(&self) -> String {
        String::new()
    }
}

#[derive(Debug, Serialize)]
pub struct PathReport {}
impl Report for PathReport {
    fn human(&self) -> String {
        String::new()
    }
}

pub fn ls(_ctx: &Ctx, _cwd: &Path, _args: &LsArgs) -> Result<LsReport> {
    todo!("worker B")
}

pub fn path(_ctx: &Ctx, _cwd: &Path, _args: &PathArgs) -> Result<PathReport> {
    todo!("worker B")
}
