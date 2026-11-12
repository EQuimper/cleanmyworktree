//! `cmw update`: find the latest release and upgrade through whatever installed this binary.
//!
//! The latest version comes from the redirect of `github.com/<repo>/releases/latest` (system
//! `curl`, no API token, no rate limit), cached for a day in `$CMW_HOME/update-check.json`.
//! Only `cmw update` and the TUI ever check; other commands stay offline.
use crate::cli::UpdateArgs;
use crate::output::{CmwError, Ctx, Report, Result, sh_quote};
use crate::paths;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const RELEASES_LATEST: &str = "https://github.com/EQuimper/cleanmyworktree/releases/latest";
const INSTALLER: &str =
    "https://github.com/EQuimper/cleanmyworktree/releases/latest/download/cmw-installer.sh";
const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// How this binary was installed, which decides how to upgrade it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Method {
    Homebrew,
    /// dist's shell installer (leaves a receipt in `~/.config/cmw`).
    Installer,
    /// `cargo install cmw` from crates.io.
    Cargo,
    /// Built from a checkout (`cargo install --path`), or unknown.
    Source {
        path: Option<String>,
    },
}

impl Method {
    pub fn name(&self) -> &'static str {
        match self {
            Method::Homebrew => "homebrew",
            Method::Installer => "installer",
            Method::Cargo => "cargo",
            Method::Source { .. } => "source",
        }
    }

    /// The upgrade command, or None when cmw can't upgrade itself.
    pub fn command(&self) -> Option<String> {
        match self {
            Method::Homebrew => Some("brew upgrade equimper/tap/cmw".into()),
            Method::Installer => Some(format!(
                "curl --proto '=https' --tlsv1.2 -LsSf {INSTALLER} | sh"
            )),
            Method::Cargo => Some("cargo install cmw --locked".into()),
            Method::Source { .. } => None,
        }
    }

    /// What to run instead when `command()` is None.
    pub fn manual_hint(&self) -> String {
        match self {
            Method::Source { path: Some(p) } => {
                let p = sh_quote(p);
                format!("git -C {p} pull && cargo install --path {p} --locked")
            }
            _ => "brew install equimper/tap/cmw".into(),
        }
    }
}

/// Detect how the running binary was installed.
pub fn detect() -> Method {
    let exe = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .unwrap_or_default();
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| paths::home().join(".config"));
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| paths::home().join(".cargo"));
    detect_from(&exe, &config, &cargo_home)
}

fn detect_from(exe: &Path, config_dir: &Path, cargo_home: &Path) -> Method {
    if exe.to_string_lossy().contains("/Cellar/cmw/") {
        return Method::Homebrew;
    }
    let in_cargo_bin = exe.starts_with(cargo_home.join("bin"));
    if in_cargo_bin && config_dir.join("cmw/cmw-receipt.json").is_file() {
        return Method::Installer;
    }
    if in_cargo_bin
        && let Ok(text) = std::fs::read_to_string(cargo_home.join(".crates2.json"))
        && let Ok(doc) = serde_json::from_str::<serde_json::Value>(&text)
        && let Some(installs) = doc["installs"].as_object()
        && let Some(key) = installs.keys().find(|k| k.starts_with("cmw "))
    {
        if key.contains("(registry+") || key.contains("(sparse+") {
            return Method::Cargo;
        }
        let path = key
            .split("(path+file://")
            .nth(1)
            .map(|rest| rest.trim_end_matches(')').to_string());
        return Method::Source { path };
    }
    Method::Source { path: None }
}

/// `"1.2.3"` → (1, 2, 3). Pre-releases and junk → None (never offered as updates).
fn parse(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim().trim_start_matches('v');
    let mut it = v.split('.');
    let parts = (
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
    );
    it.next().is_none().then_some(parts)
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    matches!((parse(latest), parse(current)), (Some(l), Some(c)) if l > c)
}

/// `…/releases/tag/v1.2.3` → `1.2.3`.
fn version_from_location(location: &str) -> Option<String> {
    let tag = location.trim().rsplit_once("/releases/tag/")?.1;
    let v = tag.trim_start_matches('v');
    parse(v).map(|_| v.to_string())
}

/// Ask GitHub which tag `releases/latest` points at. Ok(None): no release published.
fn fetch_latest() -> Result<Option<String>> {
    // Overridable for mirrors and tests.
    let url = std::env::var("CMW_UPDATE_URL").unwrap_or_else(|_| RELEASES_LATEST.into());
    let out = Command::new("curl")
        .args([
            "-sS",
            "-o",
            "/dev/null",
            "--max-time",
            "4",
            "-w",
            "%{http_code} %{redirect_url}",
            &url,
        ])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| CmwError::error(format!("cannot run curl: {e}")))?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(CmwError::error(format!("could not reach {url}: {err}")));
    }
    let (code, location) = text.split_once(' ').unwrap_or((text.as_str(), ""));
    match code {
        "301" | "302" | "303" | "307" | "308" => Ok(version_from_location(location)),
        // No release yet (or the repository isn't public).
        "404" => Ok(None),
        other => Err(CmwError::error(format!(
            "unexpected HTTP {other} from {url}"
        ))),
    }
}

#[derive(Serialize, Deserialize)]
struct Cache {
    checked_at: u64,
    latest: Option<String>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn cache_path() -> PathBuf {
    paths::cmw_home().join("update-check.json")
}

fn read_cache() -> Option<Cache> {
    let c: Cache = serde_json::from_str(&std::fs::read_to_string(cache_path()).ok()?).ok()?;
    (now().saturating_sub(c.checked_at) < CACHE_TTL.as_secs()).then_some(c)
}

fn write_cache(latest: &Option<String>) {
    let path = cache_path();
    let Some(dir) = path.parent() else {
        return;
    };
    let cache = Cache {
        checked_at: now(),
        latest: latest.clone(),
    };
    let tmp = dir.join("update-check.json.tmp");
    if std::fs::create_dir_all(dir).is_ok()
        && serde_json::to_vec(&cache).is_ok_and(|b| std::fs::write(&tmp, b).is_ok())
    {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// Latest released version. `fresh` skips the one-day cache.
pub fn latest(fresh: bool) -> Result<Option<String>> {
    if !fresh && let Some(c) = read_cache() {
        return Ok(c.latest);
    }
    let latest = fetch_latest()?;
    write_cache(&latest);
    Ok(latest)
}

/// For the TUI: a newer version, if one is known. Quiet on any failure; honors CMW_NO_UPDATE_CHECK.
pub fn available_quietly() -> Option<String> {
    if std::env::var_os("CMW_NO_UPDATE_CHECK").is_some_and(|v| !v.is_empty() && v != "0") {
        return None;
    }
    latest(false)
        .ok()
        .flatten()
        .filter(|l| is_newer(l, CURRENT))
}

#[derive(Debug, Serialize)]
pub struct UpdateReport {
    pub current: String,
    pub latest: Option<String>,
    pub update_available: bool,
    /// homebrew | installer | cargo | source
    pub method: &'static str,
    /// The upgrade command for this install, when cmw can run one.
    pub command: Option<String>,
    pub updated: bool,
    /// Version `cmw` on PATH reports after updating.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installed: Option<String>,
}

impl Report for UpdateReport {
    fn human(&self) -> String {
        match (&self.latest, self.updated) {
            (Some(l), true) => format!("updated cmw {} → {l}", self.current),
            (Some(l), false) if self.update_available => {
                format!(
                    "cmw {l} is available (you have {})\nrun: cmw update",
                    self.current
                )
            }
            (Some(_), false) => format!("cmw {} is up to date", self.current),
            (None, _) => format!("cmw {}; no published release found", self.current),
        }
    }
}

pub fn run(ctx: &Ctx, args: &UpdateArgs) -> Result<UpdateReport> {
    let method = detect();
    let latest = latest(true)?;
    let available = latest.as_deref().is_some_and(|l| is_newer(l, CURRENT));
    let mut report = UpdateReport {
        current: CURRENT.into(),
        latest: latest.clone(),
        update_available: available,
        method: method.name(),
        command: method.command(),
        updated: false,
        installed: None,
    };
    if args.check || !available {
        return Ok(report);
    }
    let Some(cmd) = method.command() else {
        return Err(CmwError::refused(
            format!(
                "cmw {CURRENT} was built from source, so it can't update itself to {}",
                latest.unwrap_or_default()
            ),
            method.manual_hint(),
        ));
    };
    ctx.progress(format!("running: {cmd}"));
    run_streaming(ctx, &cmd)?;
    report.updated = true;
    report.installed = installed_version();
    if let (Some(installed), Some(l)) = (&report.installed, &latest)
        && installed != l
    {
        ctx.warn(format!(
            "`cmw` on PATH still reports {installed}; another install may shadow the updated one"
        ));
    }
    Ok(report)
}

/// Run `cmd` through `sh`, sending each output line to progress.
fn run_streaming(ctx: &Ctx, cmd: &str) -> Result<()> {
    let mut child = Command::new("sh")
        .args(["-c", &format!("{cmd} 2>&1")])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| CmwError::error(format!("cannot run `{cmd}`: {e}")))?;
    let mut tail = Vec::new();
    if let Some(out) = child.stdout.take() {
        for line in BufReader::new(out)
            .lines()
            .map_while(std::result::Result::ok)
        {
            ctx.progress(&line);
            tail.push(line);
            if tail.len() > 20 {
                tail.remove(0);
            }
        }
    }
    let status = child
        .wait()
        .map_err(|e| CmwError::error(format!("`{cmd}` failed: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(
            CmwError::error(format!("`{cmd}` failed ({status}):\n{}", tail.join("\n")))
                .with_hint(cmd),
        )
    }
}

/// The version `cmw --version` prints for the binary first on PATH.
pub fn installed_version() -> Option<String> {
    let exe = paths::find_in_path("cmw")?;
    let out = Command::new(exe)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .last()
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("v1.0.0", "0.99.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.0.9", "0.1.0"));
        assert!(
            !is_newer("0.2.0-beta.1", "0.1.0"),
            "pre-releases are never offered"
        );
        assert!(!is_newer("garbage", "0.1.0"));
    }

    #[test]
    fn version_comes_from_the_release_redirect() {
        assert_eq!(
            version_from_location("https://github.com/o/r/releases/tag/v0.4.2"),
            Some("0.4.2".into())
        );
        assert_eq!(
            version_from_location("https://github.com/o/r/releases"),
            None
        );
        assert_eq!(version_from_location(""), None);
    }

    fn fake_home() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let cargo = t.path().join(".cargo");
        let config = t.path().join(".config");
        std::fs::create_dir_all(cargo.join("bin")).unwrap();
        std::fs::create_dir_all(&config).unwrap();
        (t, cargo, config)
    }

    #[test]
    fn detects_homebrew_by_cellar_path() {
        let (_t, cargo, config) = fake_home();
        let exe = Path::new("/opt/homebrew/Cellar/cmw/0.1.0/bin/cmw");
        assert_eq!(detect_from(exe, &config, &cargo), Method::Homebrew);
        assert_eq!(
            Method::Homebrew.command().as_deref(),
            Some("brew upgrade equimper/tap/cmw")
        );
    }

    #[test]
    fn detects_dist_installer_by_receipt() {
        let (_t, cargo, config) = fake_home();
        std::fs::create_dir_all(config.join("cmw")).unwrap();
        std::fs::write(config.join("cmw/cmw-receipt.json"), "{}").unwrap();
        let exe = cargo.join("bin/cmw");
        assert_eq!(detect_from(&exe, &config, &cargo), Method::Installer);
        assert!(
            Method::Installer
                .command()
                .unwrap()
                .contains("cmw-installer.sh | sh")
        );
    }

    #[test]
    fn detects_cargo_registry_and_path_installs() {
        let (_t, cargo, config) = fake_home();
        let exe = cargo.join("bin/cmw");
        let crates = |key: &str| {
            std::fs::write(
                cargo.join(".crates2.json"),
                format!(r#"{{"installs":{{"{key}":{{}}}}}}"#),
            )
            .unwrap();
        };
        crates("cmw 0.1.0 (registry+https://github.com/rust-lang/crates.io-index)");
        assert_eq!(detect_from(&exe, &config, &cargo), Method::Cargo);
        crates("cmw 0.1.0 (path+file:///Users/me/src/cmw)");
        let m = detect_from(&exe, &config, &cargo);
        assert_eq!(
            m,
            Method::Source {
                path: Some("/Users/me/src/cmw".into())
            }
        );
        assert_eq!(m.command(), None);
        assert_eq!(
            m.manual_hint(),
            "git -C /Users/me/src/cmw pull && cargo install --path /Users/me/src/cmw --locked"
        );
    }

    #[test]
    fn unknown_location_is_source() {
        let (_t, cargo, config) = fake_home();
        let m = detect_from(Path::new("/usr/local/bin/cmw"), &config, &cargo);
        assert_eq!(m, Method::Source { path: None });
        assert_eq!(m.manual_hint(), "brew install equimper/tap/cmw");
    }
}
