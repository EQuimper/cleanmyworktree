//! `cmw skill install|print`, `cmw hook claude`. STUB: owned by worker C.
use crate::output::{Ctx, Report, Result};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize)]
pub struct TextReport {}
impl Report for TextReport {
    fn human(&self) -> String {
        String::new()
    }
}

#[derive(Debug, Serialize)]
pub struct SkillInstallReport {}
impl Report for SkillInstallReport {
    fn human(&self) -> String {
        String::new()
    }
}

pub fn install(_ctx: &Ctx, _dir: Option<&Path>) -> Result<SkillInstallReport> {
    todo!("worker C")
}

pub fn print(_agents_md: bool) -> TextReport {
    todo!("worker C")
}

pub fn hook_claude() -> TextReport {
    todo!("worker C")
}
