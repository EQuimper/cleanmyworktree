//! Optional `.cmw.toml` at the repo root, committed so humans and agents get the same behavior.
//!
//! ```toml
//! [[rule]]
//! pattern = "ios/Secrets.xcconfig"   # glob vs repo-relative path
//! strategy = "copy"                  # clone | copy | regen | skip
//! dir = false                        # optional: only dirs (true) / only files (false)
//! reclaimable = false                # optional: clean-artifacts may delete it
//!
//! [hydrate]
//! post = ["npm run codegen"]         # run with `sh -c` in the worktree root after hydrate
//! ```

use crate::output::{CmwError, Result};
use crate::rules::{Rule, Strategy};
use serde::Deserialize;
use std::path::Path;

pub const FILE_NAME: &str = ".cmw.toml";

#[derive(Debug, Default)]
pub struct Config {
    /// User rules, checked before built-ins.
    pub rules: Vec<Rule>,
    /// Shell commands run (via `sh -c`) in the worktree root after hydrate.
    pub post_hydrate: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    rule: Vec<RawRule>,
    #[serde(default)]
    hydrate: RawHydrate,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRule {
    pattern: String,
    strategy: Strategy,
    dir: Option<bool>,
    #[serde(default)]
    reclaimable: bool,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawHydrate {
    #[serde(default)]
    post: Vec<String>,
}

/// Load `<root>/.cmw.toml`. Missing file = default config. Invalid file = usage error.
pub fn load(root: &Path) -> Result<Config> {
    let path = root.join(FILE_NAME);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(e.into()),
    };
    let raw: RawConfig = toml::from_str(&text).map_err(|e| {
        CmwError::usage(format!("invalid {}: {e}", path.display()))
            .with_hint(format!("fix or delete {}", path.display()))
    })?;
    let rules = raw
        .rule
        .into_iter()
        .map(|r| Rule {
            pattern: r.pattern,
            strategy: r.strategy,
            ecosystem: "user".to_string(),
            dir: r.dir,
            reclaimable: r.reclaimable,
            native_project: false,
        })
        .collect();
    Ok(Config {
        rules,
        post_hydrate: raw.hydrate.post,
    })
}

/// Rules for a worktree: its `.cmw.toml` (if any) layered over built-ins.
pub fn ruleset(root: &Path) -> Result<(crate::rules::RuleSet, Config)> {
    let mut cfg = load(root)?;
    let rules = std::mem::take(&mut cfg.rules);
    let set = crate::rules::RuleSet::new(rules).map_err(CmwError::usage)?;
    Ok((set, cfg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_default() {
        let tmp = tempfile::tempdir().unwrap();
        let c = load(tmp.path()).unwrap();
        assert!(c.rules.is_empty() && c.post_hydrate.is_empty());
    }

    #[test]
    fn parses_rules_and_hooks() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(FILE_NAME),
            "[[rule]]\npattern = \"ios/Secrets.xcconfig\"\nstrategy = \"copy\"\n\n[hydrate]\npost = [\"echo hi\"]\n",
        )
        .unwrap();
        let c = load(tmp.path()).unwrap();
        assert_eq!(c.rules.len(), 1);
        assert_eq!(c.rules[0].strategy, Strategy::Copy);
        assert_eq!(c.post_hydrate, vec!["echo hi".to_string()]);
    }

    #[test]
    fn unknown_keys_are_usage_errors() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(FILE_NAME), "nope = 1\n").unwrap();
        let e = load(tmp.path()).unwrap_err();
        assert_eq!(e.kind, crate::output::ErrorKind::Usage);
    }
}
