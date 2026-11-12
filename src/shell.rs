//! `cmw shell-init`. STUB: owned by worker C.
use crate::cli::ShellInitArgs;
use crate::output::Report;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ShellInitReport {}
impl Report for ShellInitReport {
    fn human(&self) -> String {
        String::new()
    }
}

pub fn init(_args: &ShellInitArgs) -> ShellInitReport {
    todo!("worker C")
}
