//! `cmw update`: find the latest release and upgrade through whatever installed this binary.
//!
//! The latest version comes from the redirect of `github.com/<repo>/releases/latest` (system
//! `curl`, no API token, no rate limit). The last answer is kept in `$CMW_HOME/update-check.json`
//! so the TUI can show it instantly while it checks again in the background.
//! Only `cmw update` and the TUI ever check; other commands stay offline.
use crate::cli::UpdateArgs;
use crate::output::{CmwError, Ctx, Report, Result, sh_quote};
use crate::paths;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

const RELEASES_LATEST: &str = "https://github.com/EQuimper/cleanmyworktree/releases/latest";
const INSTALLER: &str =
    "https://github.com/EQuimper/cleanmyworktree/releases/latest/download/cmw-installer.sh";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// How this binary was installed, which decides how to upgrade it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Method {
    Homebrew,
    /// dist's shell installer (its receipt in `~/.config/cmw` claims this binary).
    Installer,
    /// `cargo install cmw` from crates.io.
    Cargo,
    /// Built from a checkout (`cargo install --path`).
    Source {
        path: Option<String>,
    },
    /// Nothing claims this binary, or more than one installer does.
    Unknown,
}

impl Method {
    pub fn name(&self) -> &'static str {
        match self {
            Method::Homebrew => "homebrew",
            Method::Installer => "installer",
            Method::Cargo => "cargo",
            Method::Source { .. } => "source",
            Method::Unknown => "unknown",
        }
    }

    /// The upgrade command as shown to people, or None when cmw can't upgrade itself.
    pub fn command(&self) -> Option<String> {
        match self {
            Method::Homebrew => Some("brew upgrade equimper/tap/cmw".into()),
            // Download fully, then run: `curl | sh` would hide a failed download behind sh's status.
            Method::Installer => Some(format!(
                "f=$(mktemp -t cmw-installer) && curl --proto '=https' --tlsv1.2 -fLsS {INSTALLER} -o \"$f\" && sh \"$f\""
            )),
            Method::Cargo => Some("cargo install cmw --locked".into()),
            Method::Source { .. } | Method::Unknown => None,
        }
    }

    /// What actually runs: `command()`, failing on the first error and cleaning up after itself.
    fn script(&self) -> Option<String> {
        match self {
            Method::Installer => Some(format!(
                "set -e\nf=$(mktemp -t cmw-installer)\ntrap 'rm -f \"$f\"' EXIT\ncurl --proto '=https' --tlsv1.2 -fLsS {INSTALLER} -o \"$f\"\nsh \"$f\""
            )),
            other => other.command(),
        }
    }

    /// Why cmw won't run an upgrade itself.
    pub fn refusal(&self) -> String {
        match self {
            Method::Source { .. } => {
                format!("cmw {CURRENT} was built from source, so it can't update itself")
            }
            _ => format!(
                "can't tell how cmw {CURRENT} was installed (no installer, or more than one, claims it)"
            ),
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
    detect_from(&exe, &config, &cargo_home, CURRENT)
}

/// The installer receipt claims `exe`: it installed into a prefix containing it, at this version.
fn receipt_claims(config_dir: &Path, exe: &Path, current: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(config_dir.join("cmw/cmw-receipt.json")) else {
        return false;
    };
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    let prefix = doc["install_prefix"]
        .as_str()
        .is_some_and(|p| !p.is_empty() && exe.starts_with(p));
    prefix && doc["version"].as_str() == Some(current)
}

/// Cargo's record of installing cmw at this version (`cmw 0.1.0 (<source>)`).
fn cargo_record(cargo_home: &Path, current: &str) -> Option<String> {
    let text = std::fs::read_to_string(cargo_home.join(".crates2.json")).ok()?;
    let doc = serde_json::from_str::<serde_json::Value>(&text).ok()?;
    let prefix = format!("cmw {current} (");
    doc["installs"]
        .as_object()?
        .keys()
        .find(|k| k.starts_with(&prefix))
        .cloned()
}

fn detect_from(exe: &Path, config_dir: &Path, cargo_home: &Path, current: &str) -> Method {
    if exe.to_string_lossy().contains("/Cellar/cmw/") {
        return Method::Homebrew;
    }
    let receipt = receipt_claims(config_dir, exe, current);
    let cargo = exe
        .starts_with(cargo_home.join("bin"))
        .then(|| cargo_record(cargo_home, current))
        .flatten();
    match (receipt, cargo) {
        (true, None) => Method::Installer,
        (false, Some(key)) if key.contains("(registry+") || key.contains("(sparse+") => {
            Method::Cargo
        }
        (false, Some(key)) => Method::Source {
            path: key
                .split("(path+file://")
                .nth(1)
                .map(|rest| rest.trim_end_matches(')').to_string()),
        },
        // Both claim it, or neither: guessing could run the wrong installer.
        _ => Method::Unknown,
    }
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

fn read_cache(path: &Path) -> Option<Cache> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
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

/// Latest released version, asked from GitHub now. The answer is remembered for `cached_newer`.
pub fn latest() -> Result<Option<String>> {
    let latest = fetch_latest()?;
    write_cache(&latest);
    Ok(latest)
}

/// `CMW_NO_UPDATE_CHECK` set: the TUI never looks for releases.
pub fn check_disabled() -> bool {
    std::env::var_os("CMW_NO_UPDATE_CHECK").is_some_and(|v| !v.is_empty() && v != "0")
}

/// A newer version from the last check, without touching the network. Any age is fine: a release
/// that was newer than this binary then still is.
pub fn cached_newer() -> Option<String> {
    newer_in_cache(&cache_path())
}

fn newer_in_cache(path: &Path) -> Option<String> {
    read_cache(path)?.latest.filter(|l| is_newer(l, CURRENT))
}

/// A newer version, asked from GitHub now. Err when the check fails (offline, timeout).
pub fn fetch_newer() -> Result<Option<String>> {
    Ok(latest()?.filter(|l| is_newer(l, CURRENT)))
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
    /// That `cmw`: what the TUI restarts into.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installed_path: Option<PathBuf>,
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
    let latest = latest()?;
    let available = latest.as_deref().is_some_and(|l| is_newer(l, CURRENT));
    let mut report = UpdateReport {
        current: CURRENT.into(),
        latest: latest.clone(),
        update_available: available,
        method: method.name(),
        command: method.command(),
        updated: false,
        installed: None,
        installed_path: None,
    };
    if args.check || !available {
        return Ok(report);
    }
    let (Some(cmd), Some(script)) = (method.command(), method.script()) else {
        return Err(CmwError::refused(
            format!(
                "{}; {} is available",
                method.refusal(),
                latest.unwrap_or_default()
            ),
            method.manual_hint(),
        ));
    };
    let latest = latest.unwrap_or_default();
    ctx.progress(format!("running: {cmd}"));
    run_streaming(ctx, &script, &cmd)?;
    // Success means the `cmw` people will run is the new one, not just that the installer exited 0.
    let Some((path, installed)) = installed_version() else {
        return Err(
            CmwError::error("the upgrade finished, but no `cmw` was found on PATH").with_hint(cmd),
        );
    };
    report.installed = Some(installed.clone());
    report.installed_path = Some(path.clone());
    if installed != latest {
        let why = if method == Method::Homebrew {
            "the new release may not be in Homebrew yet, or another cmw earlier on PATH shadows it"
        } else {
            "another cmw earlier on PATH may shadow the updated one"
        };
        return Err(CmwError::error(format!(
            "the upgrade finished, but {} still reports {installed} ({why})",
            path.display()
        ))
        .with_hint(cmd));
    }
    report.updated = true;
    refresh_agent_skills(ctx, &path);
    Ok(report)
}

/// Let the NEW binary rewrite any cmw skill files left by an older release. Never fails the update.
fn refresh_agent_skills(ctx: &Ctx, installed: &Path) {
    let ok = Command::new(installed)
        .args(["agents", "install", "--refresh", "--json"])
        .env_remove("CMW_JSON")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        ctx.warn("couldn't refresh agent skills; run: cmw agents install");
    }
}

/// Run `script` through `sh` (stderr folded into stdout), sending each line to progress.
/// `cmd` is how the command is shown in errors.
fn run_streaming(ctx: &Ctx, script: &str, cmd: &str) -> Result<()> {
    let mut child = Command::new("sh")
        .args(["-c", &format!("exec 2>&1\n{script}")])
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

/// The `cmw` first on PATH and the version it reports.
fn installed_version() -> Option<(PathBuf, String)> {
    let exe = paths::find_in_path("cmw")?;
    let out = Command::new(&exe)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let version = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .last()?
        .to_string();
    Some((exe, version))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_newer_release_shows_at_any_age() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update-check.json");
        assert_eq!(newer_in_cache(&path), None);
        std::fs::write(&path, r#"{"checked_at":0,"latest":"99.0.0"}"#).unwrap();
        assert_eq!(newer_in_cache(&path).as_deref(), Some("99.0.0"));
        std::fs::write(&path, r#"{"checked_at":0,"latest":"0.0.1"}"#).unwrap();
        assert_eq!(newer_in_cache(&path), None);
    }

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
        std::fs::create_dir_all(config.join("cmw")).unwrap();
        (t, cargo, config)
    }

    fn receipt(config: &Path, prefix: &Path, version: &str) {
        std::fs::write(
            config.join("cmw/cmw-receipt.json"),
            format!(
                r#"{{"install_prefix":"{}","version":"{version}","binaries":["cmw"]}}"#,
                prefix.display()
            ),
        )
        .unwrap();
    }

    fn crates2(cargo: &Path, key: &str) {
        std::fs::write(
            cargo.join(".crates2.json"),
            format!(r#"{{"installs":{{"{key}":{{}}}}}}"#),
        )
        .unwrap();
    }

    #[test]
    fn detects_homebrew_by_cellar_path() {
        let (_t, cargo, config) = fake_home();
        let exe = Path::new("/opt/homebrew/Cellar/cmw/0.1.0/bin/cmw");
        assert_eq!(detect_from(exe, &config, &cargo, "0.1.0"), Method::Homebrew);
        assert_eq!(
            Method::Homebrew.command().as_deref(),
            Some("brew upgrade equimper/tap/cmw")
        );
    }

    #[test]
    fn detects_dist_installer_only_when_its_receipt_claims_this_binary() {
        let (_t, cargo, config) = fake_home();
        let exe = cargo.join("bin/cmw");
        receipt(&config, &cargo, "0.1.0");
        assert_eq!(
            detect_from(&exe, &config, &cargo, "0.1.0"),
            Method::Installer
        );
        // A receipt from another version (since replaced) doesn't count.
        assert_eq!(detect_from(&exe, &config, &cargo, "0.2.0"), Method::Unknown);
        // Nor does one for another prefix.
        receipt(&config, Path::new("/somewhere/else"), "0.1.0");
        assert_eq!(detect_from(&exe, &config, &cargo, "0.1.0"), Method::Unknown);
    }

    #[test]
    fn stale_receipt_does_not_override_a_cargo_source_install() {
        let (_t, cargo, config) = fake_home();
        let exe = cargo.join("bin/cmw");
        receipt(&config, &cargo, "0.1.0");
        crates2(&cargo, "cmw 0.2.0 (path+file:///Users/me/src/cmw)");
        assert_eq!(
            detect_from(&exe, &config, &cargo, "0.2.0"),
            Method::Source {
                path: Some("/Users/me/src/cmw".into())
            }
        );
        // Both claiming the same version: ambiguous, so cmw runs neither.
        crates2(&cargo, "cmw 0.1.0 (path+file:///Users/me/src/cmw)");
        assert_eq!(detect_from(&exe, &config, &cargo, "0.1.0"), Method::Unknown);
        assert_eq!(Method::Unknown.command(), None);
    }

    #[test]
    fn detects_cargo_registry_and_path_installs() {
        let (_t, cargo, config) = fake_home();
        let exe = cargo.join("bin/cmw");
        crates2(
            &cargo,
            "cmw 0.1.0 (registry+https://github.com/rust-lang/crates.io-index)",
        );
        assert_eq!(detect_from(&exe, &config, &cargo, "0.1.0"), Method::Cargo);
        crates2(&cargo, "cmw 0.1.0 (path+file:///Users/me/src/cmw)");
        let m = detect_from(&exe, &config, &cargo, "0.1.0");
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
    fn unknown_location_is_unknown() {
        let (_t, cargo, config) = fake_home();
        let m = detect_from(Path::new("/usr/local/bin/cmw"), &config, &cargo, "0.1.0");
        assert_eq!(m, Method::Unknown);
        assert_eq!(m.manual_hint(), "brew install equimper/tap/cmw");
    }

    /// The installer script must fail when the download fails (no `curl | sh` masking).
    #[test]
    fn installer_script_fails_when_the_download_fails() {
        let script = Method::Installer
            .script()
            .unwrap()
            .replace(INSTALLER, "http://127.0.0.1:9/cmw-installer.sh")
            .replace("--proto '=https' ", "");
        let ctx = Ctx::new(true, "test");
        let err = run_streaming(&ctx, &script, "installer").unwrap_err();
        assert!(err.message.contains("failed"), "{}", err.message);
    }
}
