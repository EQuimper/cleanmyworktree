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

    /// Run the real wrapper with a stub `cmw` that answers the way the TUI does.
    fn cd_after_tui(shell: Shell, exe: &str) -> Option<String> {
        let sh = crate::paths::find_in_path(exe)?;
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let (bin, dest) = (root.join("bin"), root.join("dest"));
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        let stub = bin.join("cmw");
        std::fs::write(
            &stub,
            format!(
                "#!/bin/sh\n[ -n \"$CMW_CD_FILE\" ] && printf %s '{}' > \"$CMW_CD_FILE\"\nexit 3\n",
                dest.display()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        let init = root.join("init");
        std::fs::write(&init, init_script(shell)).unwrap();
        let script = match shell {
            Shell::Fish => format!(
                "source {}; set -x PATH {} $PATH; cmw; echo $status; pwd",
                init.display(),
                bin.display()
            ),
            _ => format!(
                "source {}; PATH={}:$PATH; cmw; echo $?; pwd",
                init.display(),
                bin.display()
            ),
        };
        let mut cmd = std::process::Command::new(sh);
        if matches!(shell, Shell::Fish) {
            cmd.arg("--no-config");
        } else if matches!(shell, Shell::Zsh) {
            cmd.arg("-f");
        } else {
            cmd.args(["--norc", "--noprofile"]);
        }
        let out = cmd.arg("-c").arg(script).output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(
            text.ends_with(&format!("{}\n", dest.display())),
            "{exe}: {text}"
        );
        Some(text)
    }

    fn init_script(shell: Shell) -> String {
        init(&ShellInitArgs { shell }).script
    }

    #[test]
    fn bare_cmw_cds_where_the_tui_says_and_keeps_its_exit_code() {
        for (shell, exe) in [
            (Shell::Zsh, "zsh"),
            (Shell::Bash, "bash"),
            (Shell::Fish, "fish"),
        ] {
            if let Some(out) = cd_after_tui(shell, exe) {
                assert!(out.starts_with("3\n"), "{exe}: {out}");
            }
        }
    }
}
