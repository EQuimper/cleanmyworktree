//! `cmw shell-init <shell>`: prints a wrapper so `cmw cd <name>` changes the caller's directory,
//! and so does picking "Open" in the TUI (bare `cmw`): the TUI writes the chosen path to
//! `$CMW_CD_FILE` and the wrapper `cd`s there after it exits.
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
  if [ "$#" -eq 0 ]; then
    local f p rc
    f="$(mktemp -t cmw-cd.XXXXXX)" || return $?
    CMW_CD_FILE="$f" command cmw
    rc=$?
    p="$(cat -- "$f")"
    rm -f -- "$f"
    if [ -n "$p" ]; then builtin cd -- "$p" || return $?; fi
    return $rc
  elif [ "$1" = "cd" ]; then
    shift
    local p
    p="$(command cmw path "$@")" || return $?
    builtin cd -- "$p"
  else
    command cmw "$@"
  fi
}"#;

const FISH: &str = r#"function cmw
    if test (count $argv) -eq 0
        set -l f (mktemp -t cmw-cd.XXXXXX); or return $status
        CMW_CD_FILE=$f command cmw
        set -l rc $status
        set -l p (cat -- $f)
        rm -f -- $f
        if test -n "$p"
            builtin cd -- "$p"; or return $status
        end
        return $rc
    else if test "$argv[1]" = "cd"
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
    ShellInitReport {
        shell: shell.into(),
        script: script.into(),
    }
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
            assert!(r.script.contains("CMW_CD_FILE"), "{}", r.shell);
            assert_eq!(r.human(), r.script);
        }
    }
}
