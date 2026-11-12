//! Interactive UI for bare `cmw` on a terminal. Agents and scripts never see it: without a TTY on
//! both stdin and stdout, or with `--json` / `CMW_JSON=1`, bare `cmw` keeps printing help (exit 2).
mod app;
mod jobs;
#[cfg(test)]
mod tests;
mod theme;
mod ui;

use app::App;
use ratatui::crossterm::event::{self, Event};
use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

/// True when bare `cmw` should open the TUI instead of printing help.
pub fn should_launch(json: bool) -> bool {
    !json
        && std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && std::env::var("TERM").map_or(true, |t| t != "dumb")
}

/// Runs the TUI until the user quits. Returns the process exit code.
pub fn run(cwd: PathBuf) -> i32 {
    // Query the terminal's colors before raw mode / the alternate screen take over.
    let theme = theme::Theme::detect();
    let (tx, rx) = mpsc::channel();
    let mut app = App::new(cwd, tx);
    app.reload();

    let mut terminal = match ratatui::try_init() {
        Ok(t) => t,
        Err(e) => {
            ratatui::restore();
            eprintln!("error: cannot start the interactive UI: {e}");
            return 1;
        }
    };
    let result = (|| -> std::io::Result<()> {
        while !app.quit {
            terminal.draw(|f| ui::render(f, &app, &theme))?;
            if event::poll(Duration::from_millis(80))? {
                match event::read()? {
                    Event::Key(key) => app.on_key(key),
                    Event::Resize(..) => {}
                    _ => {}
                }
            }
            while let Ok(msg) = rx.try_recv() {
                app.on_msg(msg);
            }
            app.on_tick();
        }
        Ok(())
    })();
    ratatui::restore();
    if let Err(e) = result {
        eprintln!("error: {e}");
        return 1;
    }
    hand_off_directory(app.cd_on_exit);
    0
}

/// Tell the caller's shell where to go. The `shell-init` wrapper passes `$CMW_CD_FILE`; without it
/// the path goes to stdout and the clipboard as `cd <path>`.
fn hand_off_directory(dir: Option<PathBuf>) {
    let Some(dir) = dir else {
        return;
    };
    if let Some(file) = std::env::var_os("CMW_CD_FILE")
        && std::fs::write(&file, dir.as_os_str().as_encoded_bytes()).is_ok()
    {
        return;
    }
    let cmd = format!("cd {}", crate::output::sh_quote(&dir.display().to_string()));
    println!("{}", dir.display());
    if app::copy_to_clipboard(&cmd) {
        eprintln!(
            "Copied `{cmd}` to the clipboard. Setup → Shell integration makes cmw cd for you."
        );
    }
}
