//! `cmw shell-init <shell>`: prints a wrapper so `cmw cd <name>` changes the caller's directory.
use crate::cli::{Shell, ShellInitArgs};
use crate::output::Report;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ShellInitReport {
    pub shell: String,
    pub script: String,
}

impl Report for ShellInitReport {
    fn human(&self) -> String {
        self.script.clone()
    }
}

const POSIX: &str = r#"cmw() {
  if [ "$1" = "cd" ]; then
    shift
    local p
    p="$(command cmw path "$@")" || return $?
    builtin cd -- "$p"
  else
    command cmw "$@"
  fi
}"#;

const FISH: &str = r#"function cmw
    if test "$argv[1]" = "cd"
        set -l p (command cmw path $argv[2..-1])
        or return $status
        builtin cd -- "$p"
    else
        command cmw $argv
    end
end"#;

pub fn init(args: &ShellInitArgs) -> ShellInitReport {
    let (shell, script) = match args.shell {
        Shell::Zsh => ("zsh", POSIX),
        Shell::Bash => ("bash", POSIX),
        Shell::Fish => ("fish", FISH),
    };
    ShellInitReport { shell: shell.into(), script: script.into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_call_path_and_are_non_empty() {
        for shell in [Shell::Zsh, Shell::Bash, Shell::Fish] {
            let r = init(&ShellInitArgs { shell });
            assert!(!r.script.is_empty());
            assert!(r.script.contains("command cmw path"), "{}", r.shell);
            assert!(r.script.contains("command cmw"));
            assert_eq!(r.human(), r.script);
        }
    }
}
