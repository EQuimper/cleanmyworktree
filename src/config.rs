//! Optional `.cmw.toml` at the repo root. STUB: owned by worker A.
use crate::output::Result;
use crate::rules::Rule;
use std::path::Path;

#[derive(Debug, Default)]
pub struct Config {
    /// User rules, checked before built-ins.
    pub rules: Vec<Rule>,
    /// Shell commands run (via `sh -c`) in the worktree root after hydrate.
    pub post_hydrate: Vec<String>,
}

/// Load `<root>/.cmw.toml`; missing file = default config. Invalid file = usage error.
pub fn load(_root: &Path) -> Result<Config> {
    todo!("worker A")
}
