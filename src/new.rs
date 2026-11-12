//! `cmw new`. STUB: owned by worker A.
use crate::cli::NewArgs;
use crate::output::{Ctx, Report, Result};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize)]
pub struct NewReport {}

impl Report for NewReport {
    fn human(&self) -> String {
        String::new()
    }
}

pub fn run(_ctx: &Ctx, _cwd: &Path, _args: &NewArgs) -> Result<NewReport> {
    todo!("worker A")
}
