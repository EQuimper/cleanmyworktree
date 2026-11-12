//! What to do with each gitignored entry of the main checkout, and how to install deps.
//!
//! The deciding question per entry is "is it still correct after moving to another path?"
//!   clone — relocatable, copy-on-write clone (node_modules, Pods, composer vendor, ...)
//!   copy  — secrets / machine-local config, independent copy per worktree (.env, local.properties, keystores)
//!   regen — path-bound, rebuild with the ecosystem's fast tool (.venv has absolute shebangs)
//!   skip  — build outputs and caches with absolute paths (DerivedData, .cxx, build/, .next)
//!
//! Rules are data. `.cmw.toml` rules are checked before these built-ins; first match wins.

use globset::{Glob, GlobMatcher};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
    Clone,
    Copy,
    Regen,
    Skip,
}

#[derive(Debug, Clone, Serialize)]
pub struct Rule {
    /// Glob matched against the entry's repo-relative path (no trailing slash), e.g. `**/node_modules`.
    pub pattern: String,
    pub strategy: Strategy,
    /// Ecosystem label shown in reports: node, ios, android, expo, php, python, rust, ruby, flutter, env, agent, build, user.
    pub ecosystem: String,
    /// Only match directories (true) or files (false); None matches both.
    pub dir: Option<bool>,
    /// `clean-artifacts` and `rm` may delete matching entries (regenerable caches / build outputs).
    pub reclaimable: bool,
    /// Expo/React Native generated native project (CNG): clone, prune nested skip entries, and
    /// stamp app config hashes so hydrate can warn `stale_native` when they diverge.
    pub native_project: bool,
}

impl Rule {
    fn new(pattern: &str, strategy: Strategy, ecosystem: &str, dir: Option<bool>) -> Self {
        Self {
            pattern: pattern.to_string(),
            strategy,
            ecosystem: ecosystem.to_string(),
            dir,
            reclaimable: false,
            native_project: false,
        }
    }
    fn reclaimable(mut self) -> Self {
        self.reclaimable = true;
        self
    }
    fn native(mut self) -> Self {
        self.native_project = true;
        self
    }
}

/// Built-in rules for Node, iOS, Android, Expo/React Native, PHP, Python, Rust, Ruby, Flutter.
pub fn builtin() -> Vec<Rule> {
    use Strategy::*;
    const D: Option<bool> = Some(true);
    const F: Option<bool> = Some(false);
    vec![
        // --- Expo / React Native continuous native generation: whole ios/ or android/ ignored.
        Rule::new("**/ios", Clone, "expo", D).native(),
        Rule::new("**/android", Clone, "expo", D).native(),
        // --- Local agent config (Claude Code, Codex, Cursor, ...): follows every worktree so
        // agents get the same skills/agents/settings. Claude Code's own worktrees are never cloned.
        Rule::new("**/.claude/worktrees", Skip, "agent", D),
        Rule::new("**/.claude", Clone, "agent", D),
        Rule::new("**/.claude/**", Clone, "agent", D),
        Rule::new("**/.claude/**", Copy, "agent", F),
        Rule::new("**/.agents", Clone, "agent", D),
        Rule::new("**/.agents/**", Clone, "agent", D),
        Rule::new("**/.agents/**", Copy, "agent", F),
        Rule::new("**/.codex/**", Copy, "agent", F),
        Rule::new("**/.cursor/**", Copy, "agent", F),
        Rule::new("**/.grok/**", Clone, "agent", D),
        Rule::new("**/.grok/**", Copy, "agent", F),
        Rule::new("**/.gemini/**", Copy, "agent", F),
        Rule::new("**/.windsurf/**", Copy, "agent", F),
        Rule::new("**/CLAUDE.local.md", Copy, "agent", F),
        Rule::new("**/.mcp.json", Copy, "agent", F),
        // --- Dependencies (relocatable) -> clone
        Rule::new("**/node_modules", Clone, "node", D),
        Rule::new("**/.yarn/cache", Clone, "node", D),
        Rule::new("**/.pnpm-store", Clone, "node", D),
        Rule::new("**/Pods", Clone, "ios", D),
        Rule::new("**/Carthage", Clone, "ios", D),
        Rule::new("**/vendor", Clone, "php", D),
        Rule::new("**/target", Clone, "rust", D),
        // Content-addressed task caches: relocatable, safe to clone, also safe to delete.
        Rule::new("**/.turbo", Clone, "node", D).reclaimable(),
        Rule::new("**/.nx/cache", Clone, "node", D).reclaimable(),
        // --- Secrets and machine-local config -> independent copy
        Rule::new("**/.env", Copy, "env", F),
        Rule::new("**/.env.*", Copy, "env", F),
        Rule::new("**/*.env", Copy, "env", F),
        Rule::new("**/.envrc", Copy, "env", F),
        Rule::new("**/.dev.vars", Copy, "env", F),
        Rule::new("**/.npmrc", Copy, "node", F),
        Rule::new("**/auth.json", Copy, "php", F),
        Rule::new("**/master.key", Copy, "ruby", F),
        Rule::new("**/local.properties", Copy, "android", F),
        Rule::new("**/google-services.json", Copy, "android", F),
        Rule::new("**/*.keystore", Copy, "android", F),
        Rule::new("**/*.jks", Copy, "android", F),
        Rule::new("**/GoogleService-Info.plist", Copy, "ios", F),
        Rule::new("**/.xcode.env.local", Copy, "ios", F),
        Rule::new("**/*.xcconfig", Copy, "ios", F),
        Rule::new("**/*.p8", Copy, "ios", F),
        Rule::new("**/*.p12", Copy, "ios", F),
        Rule::new("**/*.pem", Copy, "env", F),
        // --- Path-bound environments -> regenerate with the ecosystem tool
        Rule::new("**/.venv", Regen, "python", D),
        Rule::new("**/venv", Regen, "python", D),
        Rule::new("**/.dart_tool", Regen, "flutter", D),
        // --- Build outputs / caches with absolute paths -> skip (and reclaimable)
        Rule::new("**/DerivedData", Skip, "ios", D).reclaimable(),
        Rule::new("**/.build", Skip, "ios", D).reclaimable(),
        Rule::new("**/xcuserdata", Skip, "ios", D),
        Rule::new("**/.cxx", Skip, "android", D).reclaimable(),
        Rule::new("**/.gradle", Skip, "android", D).reclaimable(),
        Rule::new("**/.kotlin", Skip, "android", D).reclaimable(),
        Rule::new("**/build", Skip, "build", D).reclaimable(),
        Rule::new("**/.next", Skip, "node", D).reclaimable(),
        Rule::new("**/.expo", Skip, "expo", D).reclaimable(),
        Rule::new("**/.nuxt", Skip, "node", D).reclaimable(),
        Rule::new("**/.svelte-kit", Skip, "node", D).reclaimable(),
        Rule::new("**/.parcel-cache", Skip, "node", D).reclaimable(),
        Rule::new("**/.vite", Skip, "node", D).reclaimable(),
        Rule::new("**/coverage", Skip, "build", D).reclaimable(),
        Rule::new("**/dist", Skip, "build", D),
        Rule::new("**/.DS_Store", Skip, "build", F),
        Rule::new("**/*.log", Skip, "build", F),
    ]
}

/// Compiled rule list. Build once per command with `RuleSet::new(user_rules)`.
pub struct RuleSet {
    rules: Vec<(Rule, GlobMatcher)>,
}

impl RuleSet {
    /// `user` rules (from .cmw.toml) take precedence over built-ins.
    pub fn new(user: Vec<Rule>) -> Result<Self, String> {
        let mut rules = Vec::new();
        for r in user.into_iter().chain(builtin()) {
            let m = Glob::new(&r.pattern)
                .map_err(|e| format!("invalid rule pattern '{}': {e}", r.pattern))?
                .compile_matcher();
            rules.push((r, m));
        }
        Ok(Self { rules })
    }

    #[cfg(test)]
    pub fn builtin_only() -> Self {
        Self::new(Vec::new()).expect("built-in patterns are valid")
    }

    /// First matching rule for a repo-relative path. A trailing `/` (git's collapsed
    /// directory marker) means the entry is a directory.
    pub fn classify(&self, rel: &str, is_dir: bool) -> Option<&Rule> {
        let path = rel.trim_end_matches('/');
        let is_dir = is_dir || rel.ends_with('/');
        self.rules
            .iter()
            .find(|(r, m)| r.dir.is_none_or(|d| d == is_dir) && m.is_match(path))
            .map(|(r, _)| r)
    }
}

/// How to install or refresh dependencies for one lockfile.
#[derive(Debug, Clone, Serialize)]
pub struct Installer {
    pub ecosystem: &'static str,
    /// Lockfile name (matched anywhere in the tree, tracked files only).
    pub lockfile: &'static str,
    /// Command run in the lockfile's directory. npm deliberately uses `install`, never `ci`:
    /// `npm ci` deletes node_modules and would throw away the clone.
    pub command: &'static [&'static str],
    /// Dependency directory next to the lockfile that hydrate clones. When the lockfile hash
    /// matches main and this dir was cloned, the install is skipped.
    pub dep_dir: Option<&'static str>,
    /// Takes minutes (pod install). Skipped with `--no-slow`; reported as `needs_setup`.
    pub slow: bool,
}

pub fn installers() -> &'static [Installer] {
    const fn i(
        ecosystem: &'static str,
        lockfile: &'static str,
        command: &'static [&'static str],
        dep_dir: Option<&'static str>,
        slow: bool,
    ) -> Installer {
        Installer {
            ecosystem,
            lockfile,
            command,
            dep_dir,
            slow,
        }
    }
    static LIST: &[Installer] = &[
        i(
            "node",
            "package-lock.json",
            &["npm", "install", "--no-audit", "--no-fund"],
            Some("node_modules"),
            false,
        ),
        i(
            "node",
            "pnpm-lock.yaml",
            &["pnpm", "install", "--prefer-offline"],
            Some("node_modules"),
            false,
        ),
        i(
            "node",
            "yarn.lock",
            &["yarn", "install"],
            Some("node_modules"),
            false,
        ),
        i(
            "node",
            "bun.lock",
            &["bun", "install"],
            Some("node_modules"),
            false,
        ),
        i(
            "node",
            "bun.lockb",
            &["bun", "install"],
            Some("node_modules"),
            false,
        ),
        i(
            "php",
            "composer.lock",
            &["composer", "install", "--no-interaction"],
            Some("vendor"),
            false,
        ),
        i(
            "ios",
            "Podfile.lock",
            &["pod", "install"],
            Some("Pods"),
            true,
        ),
        i("python", "uv.lock", &["uv", "sync"], Some(".venv"), false),
        i(
            "python",
            "poetry.lock",
            &["poetry", "install", "--no-interaction"],
            Some(".venv"),
            false,
        ),
        i("ruby", "Gemfile.lock", &["bundle", "install"], None, false),
        i(
            "flutter",
            "pubspec.lock",
            &["flutter", "pub", "get"],
            Some(".dart_tool"),
            false,
        ),
    ];
    LIST
}

/// Files whose content decides whether a cloned Expo/RN native project (ios/, android/) is
/// still valid. Hashed relative to the native dir's parent.
pub const NATIVE_STAMP_FILES: &[&str] = &[
    "app.json",
    "app.config.js",
    "app.config.ts",
    "app.config.mjs",
    "app.config.cjs",
    "package.json",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn s(rs: &RuleSet, p: &str) -> Option<Strategy> {
        rs.classify(p, false).map(|r| r.strategy)
    }

    #[test]
    fn classifies_common_entries() {
        let rs = RuleSet::builtin_only();
        assert_eq!(s(&rs, "node_modules/"), Some(Strategy::Clone));
        assert_eq!(s(&rs, "apps/web/node_modules/"), Some(Strategy::Clone));
        assert_eq!(s(&rs, ".env"), Some(Strategy::Copy));
        assert_eq!(s(&rs, "apps/api/.env.local"), Some(Strategy::Copy));
        assert_eq!(s(&rs, "android/local.properties"), Some(Strategy::Copy));
        assert_eq!(s(&rs, "ios/Pods/"), Some(Strategy::Clone));
        assert_eq!(s(&rs, ".build/DerivedData/"), Some(Strategy::Skip));
        assert_eq!(s(&rs, "android/app/.cxx/"), Some(Strategy::Skip));
        assert_eq!(s(&rs, ".venv/"), Some(Strategy::Regen));
        assert_eq!(s(&rs, "mobile/ios/"), Some(Strategy::Clone));
        assert!(rs.classify("mobile/ios/", false).unwrap().native_project);
        assert_eq!(s(&rs, "random-thing/"), None);
        assert_eq!(s(&rs, ".claude/agents/planner.md"), Some(Strategy::Copy));
        assert_eq!(s(&rs, ".agents/skills/harness/"), Some(Strategy::Clone));
        assert_eq!(s(&rs, ".claude/worktrees/"), Some(Strategy::Skip));
        assert_eq!(s(&rs, "CLAUDE.local.md"), Some(Strategy::Copy));
        assert_eq!(s(&rs, "mobile/.argent/secrets.env"), Some(Strategy::Copy));
    }

    #[test]
    fn dir_only_rules_ignore_files() {
        let rs = RuleSet::builtin_only();
        // A file literally named `build` is not the build/ output dir.
        assert_eq!(s(&rs, "build"), None);
    }

    #[test]
    fn user_rules_win() {
        let user = vec![Rule {
            pattern: "**/node_modules".into(),
            strategy: Strategy::Skip,
            ecosystem: "user".into(),
            dir: None,
            reclaimable: false,
            native_project: false,
        }];
        let rs = RuleSet::new(user).unwrap();
        assert_eq!(s(&rs, "node_modules/"), Some(Strategy::Skip));
    }
}
