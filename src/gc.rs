//! `cmw rm`, `cmw gc`, `cmw clean-artifacts`. STUB: owned by worker B.
use crate::cli::{CleanArtifactsArgs, GcArgs, RmArgs};
use crate::output::{Ctx, Report, Result};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize)]
pub struct RmReport {}
impl Report for RmReport {
    fn human(&self) -> String {
        String::new()
    }
}

#[derive(Debug, Serialize)]
pub struct GcReport {}
impl Report for GcReport {
    fn human(&self) -> String {
        String::new()
    }
}

#[derive(Debug, Serialize)]
pub struct CleanArtifactsReport {}
impl Report for CleanArtifactsReport {
    fn human(&self) -> String {
        String::new()
    }
}

pub fn rm(_ctx: &Ctx, _cwd: &Path, _args: &RmArgs) -> Result<RmReport> {
    todo!("worker B")
}

pub fn gc(_ctx: &Ctx, _cwd: &Path, _args: &GcArgs) -> Result<GcReport> {
    todo!("worker B")
}

pub fn clean_artifacts(_ctx: &Ctx, _cwd: &Path, _args: &CleanArtifactsArgs) -> Result<CleanArtifactsReport> {
    todo!("worker B")
}
