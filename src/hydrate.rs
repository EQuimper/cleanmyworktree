//! Hydrate a worktree from the main checkout. STUB: owned by worker A.
use crate::cli::{HydrateArgs, InstallFlags};
use crate::output::{Ctx, Report, Result};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize)]
pub struct HydrateReport {}

impl Report for HydrateReport {
    fn human(&self) -> String {
        String::new()
    }
}

/// CLI entry: resolve target (args.path or cwd) and source (args.from or main checkout).
pub fn run(_ctx: &Ctx, _cwd: &Path, _args: &HydrateArgs) -> Result<HydrateReport> {
    todo!("worker A")
}

/// Library entry used by `new`.
pub fn hydrate(_ctx: &Ctx, _source: &Path, _target: &Path, _dry_run: bool, _install: InstallFlags) -> Result<HydrateReport> {
    todo!("worker A")
}
