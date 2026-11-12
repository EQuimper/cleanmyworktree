//! `cmw env sync`. STUB: owned by worker A.
use crate::cli::EnvSyncArgs;
use crate::output::{Ctx, Report, Result};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize)]
pub struct EnvSyncReport {}

impl Report for EnvSyncReport {
    fn human(&self) -> String {
        String::new()
    }
}

pub fn sync(_ctx: &Ctx, _cwd: &Path, _args: &EnvSyncArgs) -> Result<EnvSyncReport> {
    todo!("worker A")
}
