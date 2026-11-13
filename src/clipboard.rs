//! Clipboard support through platform command-line tools.

use std::io::Write;
use std::process::{Command, Stdio};

/// Copy text to the clipboard, returning false if no clipboard tool succeeds.
pub fn copy(text: &str) -> bool {
    #[cfg(target_os = "macos")]
    let candidates: &[(&str, &[&str])] = &[("pbcopy", &[])];
    #[cfg(not(target_os = "macos"))]
    let candidates: &[(&str, &[&str])] = &[
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
    ];
    for (program, args) in candidates {
        #[cfg(not(target_os = "macos"))]
        if *program == "wl-copy" && std::env::var_os("WAYLAND_DISPLAY").is_none() {
            continue;
        }
        let Ok(mut child) = Command::new(program)
            .args(*args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue;
        };
        let wrote = child
            .stdin
            .take()
            .is_some_and(|mut s| s.write_all(text.as_bytes()).is_ok());
        if child.wait().is_ok_and(|s| s.success()) && wrote {
            return true;
        }
    }
    false
}
