//! macOS clipboard via `pbcopy`.

use std::io::Write;
use std::process::{Command, Stdio};

/// Pipe `text` to `pbcopy`. False when pbcopy is missing or fails.
pub fn copy(text: &str) -> bool {
    let Ok(mut child) = Command::new("pbcopy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let wrote = child
        .stdin
        .take()
        .is_some_and(|mut s| s.write_all(text.as_bytes()).is_ok());
    child.wait().is_ok_and(|s| s.success()) && wrote
}
