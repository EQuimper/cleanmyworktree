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
    assert!(app.busy.is_none());
    assert!(app.removing.contains(&PathBuf::from("/nowhere/wt")));
}

// Removal targets in these tests are real directories outside any repository, so the background
// job they start fails on the repo lock and deletes nothing.

fn removal(path: &str, res: crate::output::Result<crate::gc::RmReport>) -> super::jobs::Msg {
    super::jobs::Msg::Done(super::jobs::Done::Remove {
        target: target(path, Some("/")),
        force: false,
        res,
        warnings: vec![],
    })
}

fn removing(app: &mut App, paths: &[&str]) {
    app.removing.extend(paths.iter().map(PathBuf::from));
}

#[test]
fn removal_runs_in_the_background_and_others_can_start() {
    let mut app = app();
    let mut old = status("/usr", "feat/old", false);
    old.merged = Some(true);
    if let Load::Ready(rows) = &mut app.rows {
        rows.push(row(old));
    }
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char('d'));
    key(&mut app, KeyCode::Enter);
    assert!(app.modal.is_none());
    assert!(app.busy.is_none(), "no overlay while removing");
    assert!(app.removing.contains(&PathBuf::from("/tmp")));
    let text = screen_text(&app);
    assert!(
        text.contains("feat/done") && text.contains("removing"),
        "{text}"
    );
    key(&mut app, KeyCode::End);
    key(&mut app, KeyCode::Char('d'));
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.removing.len(), 2);
}

#[test]
fn a_row_being_removed_cannot_be_acted_on() {
    let mut app = app();
    removing(&mut app, &["/tmp"]);
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char('d'));
    assert!(app.modal.is_none());
    assert!(app.toast.as_ref().unwrap().0.contains("being removed"));
    key(&mut app, KeyCode::Enter);
    assert!(!app.quit);
}

#[test]
fn a_finished_removal_drops_its_row_and_keeps_the_page_and_cursor() {
    let mut app = app();
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::End);
    removing(&mut app, &["/tmp"]);
    app.on_msg(removal("/tmp", Ok(removed("/tmp"))));
    assert!(app.removing.is_empty());
    assert_eq!(app.rows.ready().map(Vec::len), Some(2));
    assert_eq!(app.selected_row().unwrap().branch_label(), "feat/cart");
    let text = screen_text(&app);
    assert!(!text.contains("Reading worktrees"), "{text}");
    assert!(
        text.contains("3.0 MB"),
        "sizes stay while refreshing:\n{text}"
    );
    assert!(app.refreshing(Screen::Worktrees));
    assert!(app.toast.as_ref().unwrap().0.starts_with("Removed"));
}

#[test]
fn removing_the_selected_row_moves_the_cursor_to_the_next_one() {
    let mut app = app();
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Down);
    removing(&mut app, &["/tmp"]);
    app.on_msg(removal("/tmp", Ok(removed("/tmp"))));
    assert_eq!(app.selected_row().unwrap().branch_label(), "feat/cart");
}

#[test]
fn a_load_that_started_before_a_removal_does_not_bring_it_back() {
    use crate::inventory::{LsReport, RepoStatus};
    let mut app = app();
    removing(&mut app, &["/tmp", "/usr"]);
    // `/usr` is still going, so no new load replaces the one in flight.
    app.on_msg(removal("/tmp", Ok(removed("/tmp"))));
    let manifest = env!("CARGO_MANIFEST_DIR");
    let report = LsReport::from_repos(vec![RepoStatus {
        repo: "shop".into(),
        common_dir: PathBuf::from("/.git"),
        default_branch: "main".into(),
        worktrees: vec![
            status("/", "main", true),
            status("/tmp", "feat/done", false),
            status(manifest, "feat/cart", false),
        ],
    }]);
    app.on_msg(super::jobs::Msg::Ls {
        generation: app.generation,
        res: Ok(report),
    });
    assert!(!app.refreshing(Screen::Worktrees));
    let rows = app.rows.ready().unwrap();
    let paths: Vec<_> = rows.iter().map(|r| r.st.path.clone()).collect();
    assert_eq!(paths, [PathBuf::from("/"), PathBuf::from(manifest)]);
    assert!(rows.iter().all(|r| r.size == Some(3 * 1024 * 1024)));
}

#[test]
fn a_refusal_waits_for_the_dialog_on_screen() {
    let mut app = app();
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char('d'));
    removing(&mut app, &["/nowhere/wt"]);
    app.on_msg(removal(
        "/nowhere/wt",
        Err(crate::output::CmwError::refused(
            "3 uncommitted changes in /nowhere/wt",
            "cmw rm feat/cart --force",
        )),
    ));
    assert!(matches!(app.modal, Some(Modal::Confirm { .. })));
    key(&mut app, KeyCode::Esc);
    assert!(matches!(app.modal, Some(Modal::TypeToConfirm { .. })));
}

#[test]
fn a_removal_finishing_leaves_the_running_task_alone() {
    use super::jobs::{Done, Msg};
    let mut app = app();
    app.busy = Some(super::app::Busy {
        label: "Hydrating".into(),
        started: std::time::Instant::now(),
        lines: vec![],
    });
    removing(&mut app, &["/tmp", "/usr"]);
    app.on_msg(removal("/tmp", Ok(removed("/tmp"))));
    assert!(app.busy.is_some());
    app.on_msg(removal(
        "/usr",
        Err(crate::output::CmwError::error("disk on fire")),
    ));
    assert!(app.modal.is_none(), "errors wait for the task");
    app.on_msg(Msg::Done(Done::Text {
        title: "Hydrated feat/cart".into(),
        res: Ok("done".into()),
    }));
    assert!(app.busy.is_none());
    assert!(screen_text(&app).contains("Hydrated feat/cart"));
    key(&mut app, KeyCode::Esc);
    assert!(screen_text(&app).contains("disk on fire"));
}

#[test]
fn quit_waits_for_removals_to_finish() {
    let mut app = app();
    removing(&mut app, &["/tmp"]);
    key(&mut app, KeyCode::Char('q'));
    assert!(app.quit && !app.exiting());
    assert!(screen_text(&app).contains("Finishing 1 removal"));
    app.on_msg(removal("/tmp", Ok(removed("/tmp"))));
    assert!(app.exiting());
}

#[test]
fn ctrl_c_quits_without_waiting_for_removals() {
    let mut app = app();
    removing(&mut app, &["/tmp"]);
    key(&mut app, KeyCode::Char('q'));
    app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(app.exiting());
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
            installed_path: Some(PathBuf::from("/opt/homebrew/bin/cmw")),
        }),
        warnings: vec![],
    }));
    assert!(app.quit);
    assert_eq!(app.restart, Some(PathBuf::from("/opt/homebrew/bin/cmw")));
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

#[test]
fn wheel_keeps_the_filter_open_so_typing_stays_text() {
    use ratatui::crossterm::event::{MouseEvent, MouseEventKind};
    let mut app = app();
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "feat");
    app.on_mouse(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 10,
        row: 10,
        modifiers: KeyModifiers::NONE,
    });
    assert!(app.filtering);
    assert_eq!(app.wt_sel, 1);
    // `d` is part of the filter text, not Remove.
    key(&mut app, KeyCode::Char('d'));
    assert!(app.modal.is_none());
    assert_eq!(app.filter, "featd");
}

#[test]
fn confirm_button_hits_match_the_drawn_footer_at_narrow_widths() {
    // A narrow terminal makes the Clean dialog's long sentence wrap at word boundaries.
    let mut app = app();
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char('d'));
    let mut term = Terminal::new(TestBackend::new(46, 20)).unwrap();
    let theme = Theme::new(Mode::Dark, true, false);
    term.draw(|f| ui::render(f, &app, &theme)).unwrap();
    let buf = term.backend().buffer().clone();
    for (rect, hit) in app.hits.borrow().iter() {
        if let super::app::Hit::Key(_) = hit {
            let drawn: String = (rect.x..rect.x + rect.width)
                .map(|x| buf[(x, rect.y)].symbol().to_string())
                .collect();
            assert!(
                drawn.starts_with("Enter") || drawn.starts_with("Esc"),
                "hit {hit:?} over {drawn:?}"
            );
        }
    }
}

#[test]
fn setup_lists_agent_rows() {
    let app = app();
    let titles: Vec<_> = app.setup.iter().map(|r| r.title).collect();
    assert_eq!(
        titles,
        [
            "Shell integration",
            "Agent skills",
            "Claude Code hooks",
            "Agent setup prompt",
            "AGENTS.md snippet"
        ]
    );
}

#[test]
fn opening_a_worktree_survives_removing_the_launch_one() {
    let mut app = app();
    let main = env!("CARGO_MANIFEST_DIR");
    app.cwd = PathBuf::from("/gone/wt/src");
    removing(&mut app, &["/gone/wt"]);
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.cd_on_exit, Some(PathBuf::from("/tmp")));
    app.on_msg(super::jobs::Msg::Done(super::jobs::Done::Remove {
        target: target("/gone/wt", Some(main)),
        force: false,
        res: Ok(removed("/gone/wt")),
        warnings: vec![],
    }));
    assert_eq!(app.cwd, PathBuf::from(main));
    assert_eq!(app.cd_on_exit, Some(PathBuf::from("/tmp")));
    assert!(app.exiting());
}

#[test]
fn sizes_awaiting_a_measurement_outlive_a_superseded_load() {
    use super::jobs::Msg;
    use crate::inventory::{LsReport, RepoStatus};
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a"), dir.path().join("b"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let (a_s, b_s) = (a.to_str().unwrap(), b.to_str().unwrap());
    let ls = |app: &mut App| {
        let report = LsReport::from_repos(vec![RepoStatus {
            repo: "shop".into(),
            common_dir: PathBuf::from("/.git"),
            default_branch: "main".into(),
            worktrees: vec![status(a_s, "main", true), status(b_s, "feat/b", false)],
        }]);
        app.on_msg(Msg::Ls {
            generation: app.generation,
            res: Ok(report),
        });
    };
    let mut app = app();
    app.rows = Load::Ready(vec![
        row(status(a_s, "main", true)),
        row(status(b_s, "feat/b", false)),
    ]);
    // Hydrate finished: every size may have changed.
    app.refresh(true);
    ls(&mut app);
    app.on_msg(Msg::Size {
        generation: app.generation,
        path: a.clone(),
        bytes: 5,
    });
    // The last removal finishes before `b` is measured, superseding that load.
    app.refresh(false);
    ls(&mut app);
    assert!(app.unmeasured.contains(&b), "b still needs measuring");
    assert!(!app.unmeasured.contains(&a));
    assert_eq!(app.size_of(&a), Some(5));
}

#[test]
fn outcomes_after_quitting_are_kept_for_the_terminal() {
    let mut app = app();
    removing(&mut app, &["/tmp", "/usr", "/nowhere/wt"]);
    key(&mut app, KeyCode::Char('q'));
    app.on_msg(removal(
        "/usr",
        Err(crate::output::CmwError::error("disk on fire")),
    ));
    app.on_msg(removal(
        "/nowhere/wt",
        Err(crate::output::CmwError::refused(
            "3 uncommitted changes in /nowhere/wt",
            "cmw rm feat/cart --force",
        )),
    ));
    assert!(app.modal.is_none(), "no dialog nobody can answer");
    assert!(!app.exiting());
    app.on_msg(removal("/tmp", Ok(removed("/tmp"))));
    assert!(app.exiting());
    let notes = app.exit_notes().join("\n");
    assert!(notes.contains("disk on fire"), "{notes}");
    assert!(
        notes.contains("Kept feat/cart: 3 uncommitted changes"),
        "{notes}"
    );
    assert!(notes.contains("cmw rm /nowhere/wt --force"), "{notes}");
}

#[test]
fn a_restart_after_update_keeps_queued_removal_errors() {
    use super::jobs::{Done, Msg};
    let mut app = app();
    app.busy = Some(super::app::Busy {
        label: "Updating".into(),
        started: std::time::Instant::now(),
        lines: vec![],
    });
    removing(&mut app, &["/usr"]);
    app.on_msg(removal(
        "/usr",
        Err(crate::output::CmwError::error("disk on fire")),
    ));
    app.on_msg(Msg::Done(Done::Update {
        res: Ok(crate::update::UpdateReport {
            current: "0.1.0".into(),
            latest: Some("0.2.0".into()),
            update_available: true,
            method: "homebrew",
            command: None,
            updated: true,
            installed: Some("0.2.0".into()),
            installed_path: Some(PathBuf::from("/opt/homebrew/bin/cmw")),
        }),
        warnings: vec![],
    }));
    assert!(app.exiting());
    assert!(app.exit_notes().join("\n").contains("disk on fire"));
}

#[test]
fn ctrl_c_notes_removals_it_cut_short() {
    let mut app = app();
    removing(&mut app, &["/tmp"]);
    let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
    app.on_key(ctrl_c);
    app.on_key(ctrl_c);
    assert!(app.exiting());
    assert!(app.exit_notes()[0].starts_with("Quit while removing /tmp"));
}
