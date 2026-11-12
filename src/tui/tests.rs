//! Screen and input tests: render into a `TestBackend` and assert on the text.
use super::app::{App, Load, Modal, Row, Screen};
use super::theme::{Mode, Theme};
use super::ui;
use crate::inventory::WorktreeStatus;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;
use std::sync::mpsc;

fn status(path: &str, branch: &str, is_main: bool) -> WorktreeStatus {
    WorktreeStatus {
        path: PathBuf::from(path),
        branch: Some(branch.into()),
        head: None,
        is_main,
        bare: false,
        detached: false,
        locked: false,
        prunable: false,
        dirty: 0,
        has_submodules: false,
        upstream: None,
        ahead: None,
        behind: None,
        unpushed: 0,
        merged: None,
        merged_via: None,
        merge_reason: String::new(),
        last_commit_unix: None,
        idle_days: Some(2),
        hydrated: true,
        apparent_bytes: None,
        error: None,
    }
}

fn row(st: WorktreeStatus) -> Row {
    Row {
        repo: "shop".into(),
        default_branch: "main".into(),
        repo_main: Some(PathBuf::from("/")),
        st,
        size: Some(3 * 1024 * 1024),
        here: false,
    }
}

/// An app outside any repository (no background jobs started) with a repo pretended in.
fn app() -> App {
    let dir = tempfile::tempdir().unwrap();
    let (tx, _rx) = mpsc::channel();
    let mut app = App::new(dir.path().to_path_buf(), tx);
    app.repo_name = Some("shop".into());
    app.branch = Some("feat/cart".into());
    app.all = false;
    // Real directories so rows count as usable.
    let main = status("/", "main", true);
    let mut merged = status("/tmp", "feat/done", false);
    merged.merged = Some(true);
    let mut dirty = status(env!("CARGO_MANIFEST_DIR"), "feat/cart", false);
    dirty.dirty = 3;
    app.rows = Load::Ready(vec![row(main), row(merged), row(dirty)]);
    app
}

fn key(app: &mut App, code: KeyCode) {
    app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn typed(app: &mut App, s: &str) {
    for c in s.chars() {
        key(app, KeyCode::Char(c));
    }
}

fn screen_text(app: &App) -> String {
    let mut term = Terminal::new(TestBackend::new(100, 30)).unwrap();
    let theme = Theme::new(Mode::Dark, true, false);
    term.draw(|f| ui::render(f, app, &theme)).unwrap();
    let buf = term.backend().buffer();
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn home_shows_logo_scope_and_menu() {
    let app = app();
    let text = screen_text(&app);
    assert!(text.contains("██████"), "{text}");
    assert!(text.contains("shop"));
    assert!(text.contains("feat/cart"));
    for item in ["1. Worktrees", "2. New", "3. Clean", "4. Slim", "5. Setup"] {
        assert!(text.contains(item), "missing {item}:\n{text}");
    }
    assert!(text.contains("2 linked"), "{text}");
    assert!(text.contains("Q Quit"));
}

#[test]
fn new_is_hidden_outside_a_repository() {
    let mut app = app();
    app.repo_name = None;
    let text = screen_text(&app);
    assert!(!text.contains("New"), "{text}");
    assert!(text.contains("2. Clean"));
}

#[test]
fn number_keys_open_screens_and_esc_returns() {
    let mut app = app();
    key(&mut app, KeyCode::Char('1'));
    assert_eq!(app.screen, Screen::Worktrees);
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Home);
    key(&mut app, KeyCode::Char('5'));
    assert_eq!(app.screen, Screen::Setup);
    key(&mut app, KeyCode::Char('q'));
    assert!(app.quit);
}

#[test]
fn worktrees_table_shows_state() {
    let mut app = app();
    key(&mut app, KeyCode::Enter);
    let text = screen_text(&app);
    assert!(text.contains("cmw › Worktrees"), "{text}");
    assert!(text.contains("✓ merged"), "{text}");
    assert!(text.contains("● 3 changed"), "{text}");
    assert!(text.contains("3.0 MB"), "{text}");
}

#[test]
fn filter_narrows_rows_and_typing_does_not_trigger_keys() {
    let mut app = app();
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('/'));
    // `q` and `d` are commands outside the filter; inside it they are text.
    typed(&mut app, "done");
    assert!(!app.quit);
    assert!(app.modal.is_none());
    assert_eq!(app.visible().len(), 1);
    assert_eq!(app.selected_row().unwrap().branch_label(), "feat/done");
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.visible().len(), 3);
}

#[test]
fn main_checkout_cannot_be_removed() {
    let mut app = app();
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('d'));
    assert!(app.modal.is_none());
    assert!(app.toast.as_ref().unwrap().0.contains("main checkout"));
}

#[test]
fn remove_asks_for_confirmation_and_esc_cancels() {
    let mut app = app();
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char('d'));
    assert!(matches!(app.modal, Some(Modal::Confirm { .. })));
    assert!(screen_text(&app).contains("Remove feat/done?"));
    key(&mut app, KeyCode::Esc);
    assert!(app.modal.is_none());
    assert!(app.busy.is_none());
}

#[test]
fn enter_opens_the_selected_worktree() {
    let mut app = app();
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    assert!(app.quit);
    assert_eq!(app.cd_on_exit, Some(PathBuf::from("/tmp")));
}

#[test]
fn new_form_types_branch_and_requires_one() {
    let mut app = app();
    key(&mut app, KeyCode::Char('2'));
    assert_eq!(app.screen, Screen::New);
    key(&mut app, KeyCode::Enter);
    assert!(app.busy.is_none(), "empty branch must not start a job");
    typed(&mut app, "feat/qa");
    assert_eq!(app.new_branch, "feat/qa");
    let text = screen_text(&app);
    assert!(text.contains("feat-qa"), "{text}");
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Home);
}

#[test]
fn ctrl_c_while_busy_needs_a_second_press() {
    let mut app = app();
    app.busy = Some(super::app::Busy {
        label: "Hydrating".into(),
        started: std::time::Instant::now(),
        lines: vec!["running: npm ci".into()],
    });
    assert!(screen_text(&app).contains("running: npm ci"));
    let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
    app.on_key(ctrl_c);
    assert!(!app.quit);
    app.on_key(ctrl_c);
    assert!(app.quit);
}

#[test]
fn tiny_terminal_gets_a_hint_instead_of_a_broken_layout() {
    let app = app();
    let mut term = Terminal::new(TestBackend::new(40, 8)).unwrap();
    let theme = Theme::new(Mode::Dark, false, false);
    term.draw(|f| ui::render(f, &app, &theme)).unwrap();
    let buf = term.backend().buffer();
    let text: String = buf.content().iter().map(|c| c.symbol()).collect();
    assert!(text.contains("bigger"), "{text}");
}

fn removed(path: &str) -> crate::gc::RmReport {
    crate::gc::RmReport {
        path: PathBuf::from(path),
        branch: Some("feat/cart".into()),
        dry_run: false,
        removed: true,
        branch_deleted: false,
        branch_kept_reason: None,
        derived_data_removed: vec![],
        watchman_unwatched: false,
        admin_cleanup: String::new(),
    }
}

fn target(path: &str, repo_main: Option<&str>) -> super::jobs::Target {
    super::jobs::Target {
        path: PathBuf::from(path),
        branch: Some("feat/cart".into()),
        repo_main: repo_main.map(PathBuf::from),
    }
}

#[test]
fn removing_the_launch_worktree_moves_to_the_main_checkout() {
    use super::jobs::{Done, Msg};
    let mut app = app();
    let main = env!("CARGO_MANIFEST_DIR");
    app.cwd = PathBuf::from("/gone/wt/src");
    app.on_msg(Msg::Done(Done::Remove {
        target: target("/gone/wt", Some(main)),
        force: true,
        res: Ok(removed("/gone/wt")),
        warnings: vec![],
    }));
    assert_eq!(app.cwd, PathBuf::from(main));
    assert_eq!(app.cd_on_exit, Some(PathBuf::from(main)));
}

#[test]
fn removing_another_worktree_keeps_cwd() {
    use super::jobs::{Done, Msg};
    let mut app = app();
    let before = app.cwd.clone();
    app.on_msg(Msg::Done(Done::Remove {
        target: target("/elsewhere/wt", Some("/")),
        force: false,
        res: Ok(removed("/elsewhere/wt")),
        warnings: vec![],
    }));
    assert_eq!(app.cwd, before);
    assert_eq!(app.cd_on_exit, None);
}

#[test]
fn refused_remove_needs_the_exact_branch_name_to_force() {
    use super::jobs::{Done, Msg};
    let mut app = app();
    app.on_msg(Msg::Done(Done::Remove {
        target: target("/nowhere/wt", None),
        force: false,
        res: Err(crate::output::CmwError::refused(
            "3 uncommitted changes in /nowhere/wt",
            "cmw rm feat/cart --force",
        )),
        warnings: vec![],
    }));
    assert!(matches!(app.modal, Some(Modal::TypeToConfirm { .. })));
    typed(&mut app, "feat/car");
    key(&mut app, KeyCode::Enter);
    assert!(app.busy.is_none(), "a near miss must not force anything");
    typed(&mut app, "t");
    key(&mut app, KeyCode::Enter);
    assert!(app.modal.is_none());
    assert_eq!(app.busy.as_ref().unwrap().label, "Removing feat/cart");
}

/// Render, then return the cell where `needle` starts.
fn locate(app: &App, needle: &str) -> (u16, u16) {
    let mut term = Terminal::new(TestBackend::new(100, 30)).unwrap();
    let theme = Theme::new(Mode::Dark, true, false);
    term.draw(|f| ui::render(f, app, &theme)).unwrap();
    let buf = term.backend().buffer();
    for y in 0..buf.area.height {
        let cells: Vec<String> = (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol().to_string())
            .collect();
        let line: String = cells.concat();
        if let Some(byte) = line.find(needle) {
            // Map the byte offset back to a column.
            let mut acc = 0;
            for (x, c) in cells.iter().enumerate() {
                if acc >= byte {
                    return (x as u16, y);
                }
                acc += c.len();
            }
        }
    }
    panic!("{needle:?} not on screen:\n{}", screen_text(app));
}

fn click(app: &mut App, needle: &str) {
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let (column, row) = locate(app, needle);
    app.on_mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    });
}

#[test]
fn update_pill_and_shortcut_appear_only_when_a_release_is_newer() {
    let mut app = app();
    assert!(!screen_text(&app).contains("available"));
    app.update = Some("9.9.9".into());
    let text = screen_text(&app);
    assert!(text.contains("↑ 9.9.9 available"), "{text}");
    assert!(text.contains("U Update"), "{text}");
}

#[test]
fn clicking_the_update_pill_or_pressing_u_asks_first() {
    let mut app = app();
    app.update = Some("9.9.9".into());
    click(&mut app, "↑ 9.9.9 available");
    // Test binaries aren't installed by brew/installer, so this explains how to update by hand;
    // either way nothing runs without a confirmation.
    assert!(app.modal.is_some());
    assert!(app.busy.is_none());
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Char('u'));
    assert!(app.modal.is_some());
}

#[test]
fn a_finished_update_restarts_the_tui() {
    use super::jobs::{Done, Msg};
    let mut app = app();
    app.on_msg(Msg::Done(Done::Update {
        res: Ok(crate::update::UpdateReport {
            current: "0.1.0".into(),
            latest: Some("0.2.0".into()),
            update_available: true,
            method: "homebrew",
            command: Some("brew upgrade equimper/tap/cmw".into()),
            updated: true,
            installed: Some("0.2.0".into()),
        }),
        warnings: vec![],
    }));
    assert!(app.quit && app.restart);
}

#[test]
fn clicking_a_menu_item_opens_it_and_esc_back_returns() {
    let mut app = app();
    click(&mut app, "3. Clean");
    assert_eq!(app.screen, Screen::Clean);
    click(&mut app, "Esc Back");
    assert_eq!(app.screen, Screen::Home);
}

#[test]
fn click_selects_a_row_and_a_second_click_opens_it() {
    let mut app = app();
    key(&mut app, KeyCode::Enter);
    click(&mut app, "feat/done");
    assert_eq!(app.wt_sel, 1);
    assert!(!app.quit);
    click(&mut app, "feat/done");
    assert!(app.quit);
    assert_eq!(app.cd_on_exit, Some(PathBuf::from("/tmp")));
}

#[test]
fn wheel_scrolls_lists() {
    use ratatui::crossterm::event::{MouseEvent, MouseEventKind};
    let mut app = app();
    key(&mut app, KeyCode::Enter);
    let wheel = |app: &mut App, kind| {
        app.on_mouse(MouseEvent {
            kind,
            column: 10,
            row: 10,
            modifiers: KeyModifiers::NONE,
        })
    };
    wheel(&mut app, MouseEventKind::ScrollDown);
    wheel(&mut app, MouseEventKind::ScrollDown);
    assert_eq!(app.wt_sel, 2);
    wheel(&mut app, MouseEventKind::ScrollUp);
    assert_eq!(app.wt_sel, 1);
}

#[test]
fn modal_buttons_are_clickable_and_cover_the_page() {
    let mut app = app();
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char('d'));
    // The rows under the dialog must not react while it is open.
    screen_text(&app);
    assert!(
        !app.hits
            .borrow()
            .iter()
            .any(|(_, h)| matches!(h, super::app::Hit::Row(_)))
    );
    click(&mut app, "Esc Cancel");
    assert!(app.modal.is_none());
    assert!(app.busy.is_none());
}
