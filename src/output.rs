//! Output contract shared by every command.
//!
//! Human mode: result on stdout, warnings and errors on stderr.
//! JSON mode (`--json` or `CMW_JSON=1`): exactly one JSON document on stdout:
//!   success: {"ok":true,"command":"<name>","data":{...},"warnings":[...]}
//!   failure: {"ok":false,"command":"<name>","error":{"kind":"refused","message":"...","hint":"cmw rm foo --force"},"warnings":[...]}
//! Exit codes are stable (see `ErrorKind::exit_code`).

use serde::Serialize;
use std::cell::RefCell;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// Generic failure (git failed, IO error, ...). Exit 1.
    Error,
    /// Bad arguments or invalid config. Exit 2.
    Usage,
    /// Refused because the action is unsafe (dirty tree, unpushed commits, main worktree). Exit 3.
    Refused,
    /// Worktree, repo or file not found. Exit 4.
    NotFound,
}

impl ErrorKind {
    pub fn exit_code(self) -> i32 {
        match self {
            ErrorKind::Error => 1,
            ErrorKind::Usage => 2,
            ErrorKind::Refused => 3,
            ErrorKind::NotFound => 4,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CmwError {
    pub kind: ErrorKind,
    pub message: String,
    /// Exact command that resolves the error, when one exists. Agents can run it verbatim.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl CmwError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            hint: None,
        }
    }
    pub fn error(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Error, message)
    }
    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Usage, message)
    }
    pub fn refused(message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Refused,
            message: message.into(),
            hint: Some(hint.into()),
        }
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::NotFound, message)
    }
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

impl fmt::Display for CmwError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl From<std::io::Error> for CmwError {
    fn from(e: std::io::Error) -> Self {
        let kind = if e.kind() == std::io::ErrorKind::NotFound {
            ErrorKind::NotFound
        } else {
            ErrorKind::Error
        };
        CmwError::new(kind, e.to_string())
    }
}

impl From<anyhow::Error> for CmwError {
    fn from(e: anyhow::Error) -> Self {
        CmwError::error(format!("{e:#}"))
    }
}

pub type Result<T> = std::result::Result<T, CmwError>;

/// A command result. `human()` is what a person sees; the struct itself is the JSON `data`.
pub trait Report: Serialize {
    fn human(&self) -> String;
}

/// Per-invocation context. Warnings are buffered so JSON mode can put them in the envelope.
pub struct Ctx {
    pub json: bool,
    pub command: &'static str,
    warnings: RefCell<Vec<String>>,
}

impl Ctx {
    pub fn new(json: bool, command: &'static str) -> Self {
        Self {
            json,
            command,
            warnings: RefCell::new(Vec::new()),
        }
    }

    /// Non-fatal problem. Printed to stderr immediately in human mode, collected in JSON mode.
    pub fn warn(&self, msg: impl Into<String>) {
        let msg = msg.into();
        if !self.json {
            eprintln!("warning: {msg}");
        }
        self.warnings.borrow_mut().push(msg);
    }

    /// Progress line for humans only (stderr). Silent in JSON mode.
    pub fn progress(&self, msg: impl AsRef<str>) {
        if !self.json {
            eprintln!("{}", msg.as_ref());
        }
    }

    pub fn emit<T: Report>(&self, report: &T) {
        if self.json {
            let doc = serde_json::json!({
                "ok": true,
                "command": self.command,
                "data": report,
                "warnings": *self.warnings.borrow(),
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&doc).expect("serializable report")
            );
        } else {
            let text = report.human();
            if !text.is_empty() {
                println!("{}", text.trim_end());
            }
        }
    }

    /// Prints the error and returns the process exit code.
    pub fn emit_error(&self, err: &CmwError) -> i32 {
        if self.json {
            let doc = serde_json::json!({
                "ok": false,
                "command": self.command,
                "error": err,
                "warnings": *self.warnings.borrow(),
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&doc).expect("serializable error")
            );
        } else {
            eprintln!("error: {}", err.message);
            if let Some(hint) = &err.hint {
                eprintln!("  fix: {hint}");
            }
        }
        err.kind.exit_code()
    }
}

/// Human-readable byte size (base 1024).
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

/// Shell-quote a path for hint commands.
pub fn sh_quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:@+".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}
