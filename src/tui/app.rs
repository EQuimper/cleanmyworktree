//! TUI state and input handling. Rendering lives in `ui.rs`, background work in `jobs.rs`.
use super::jobs::{self, Done, Msg, Target};
use crate::gc::{CleanArtifactsReport, GcReport};
use crate::inventory::WorktreeStatus;
use crate::output::{CmwError, Ctx, ErrorKind, human_bytes};
use crate::{git, skill};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Home,
    Worktrees,
    New,
    Clean,
    Slim,
    Setup,
}

pub struct MenuItem {
    pub screen: Screen,
    pub label: &'static str,
    pub desc: &'static str,
}

const MENU: [MenuItem; 5] = [
    MenuItem {
        screen: Screen::Worktrees,
        label: "Worktrees",
        desc: "Browse, open, hydrate, remove",
    },
    MenuItem {
        screen: Screen::New,
        label: "New",
        desc: "Create a hydrated worktree",
    },
    MenuItem {
        screen: Screen::Clean,
        label: "Clean",
        desc: "Remove merged worktrees",
    },
    MenuItem {
        screen: Screen::Slim,
        label: "Slim",
        desc: "Delete build caches from idle worktrees",
    },
    MenuItem {
        screen: Screen::Setup,
        label: "Setup",
        desc: "Shell integration, agent skill, hooks",
    },
];

/// Idle thresholds Slim cycles through with ← →.
pub const IDLE_STEPS: [u64; 6] = [0, 1, 3, 7, 14, 30];

pub enum Load<T> {
    Loading,
    Ready(T),
    Failed(String),
}

impl<T> Load<T> {
    pub fn ready(&self) -> Option<&T> {
        match self {
            Load::Ready(t) => Some(t),
            _ => None,
        }
    }
}

pub struct Row {
    pub repo: String,
    pub default_branch: String,
    /// Main checkout of the row's repo, if known.
    pub repo_main: Option<PathBuf>,
    pub st: WorktreeStatus,
    pub size: Option<u64>,
    /// The worktree `cmw` was started in.
    pub here: bool,
}

impl Row {
    pub fn target(&self) -> Target {
        Target {
            path: self.st.path.clone(),
            branch: self.st.branch.clone(),
            repo_main: self.repo_main.clone(),
        }
    }

    pub fn branch_label(&self) -> String {
        match (&self.st.branch, self.st.bare, self.st.detached) {
            (_, true, _) => "(bare)".into(),
            (Some(b), _, _) => b.clone(),
            (None, _, true) => "(detached)".into(),
            _ => "-".into(),
        }
    }

    /// Usable as a directory to open or act on.
    pub fn usable(&self) -> bool {
        !self.st.bare && !self.st.prunable && self.st.path.is_dir()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Ok,
    Warn,
    Error,
}

pub enum Action {
    Remove { target: Target, force: bool },
    Clean { picked: Vec<Target> },
    Slim { idle: u64 },
    InstallShell { rc: PathBuf, line: String },
    InstallClaudeHooks,
    Update,
}

/// Something clickable, recorded by the renderer each frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// Acts like pressing this key (footer shortcuts, modal buttons).
    Key(KeyCode),
    /// Home menu entry.
    Menu(usize),
    /// Row `n` of the current screen's list.
    Row(usize),
    UpdatePill,
}

pub enum Modal {
    Confirm {
        title: String,
        lines: Vec<String>,
        danger: bool,
        action: Action,
    },
    /// Destructive and irreversible: the user types `expect` to proceed.
    TypeToConfirm {
        title: String,
        lines: Vec<String>,
        expect: String,
        input: String,
        action: Action,
    },
    Result {
        title: String,
        text: String,
        tone: Tone,
        /// Enter opens this worktree (after `New`).
        open: Option<PathBuf>,
        scroll: u16,
    },
}

pub struct Busy {
    pub label: String,
    pub started: Instant,
    pub lines: Vec<String>,
}

pub struct SetupRow {
    pub title: &'static str,
    pub status: String,
    /// None for rows that are actions rather than state.
    pub tone: Option<Tone>,
}

pub struct App {
    pub cwd: PathBuf,
    pub repo_name: Option<String>,
    pub branch: Option<String>,
    pub all: bool,
    pub screen: Screen,
    pub home_sel: usize,
    pub generation: u64,
    pub rows: Load<Vec<Row>>,
    pub wt_sel: usize,
    pub filter: String,
    pub filtering: bool,
    pub gc: Load<GcReport>,
    pub gc_checked: Vec<bool>,
    pub gc_sel: usize,
    pub slim_idle: u64,
    pub slim: Load<CleanArtifactsReport>,
    pub slim_sel: usize,
    pub new_branch: String,
    pub new_base: String,
    pub new_focus: usize,
    /// What `cmw new` branches from when Base is empty.
    pub default_base: String,
    pub setup: Vec<SetupRow>,
    pub setup_sel: usize,
    pub modal: Option<Modal>,
    pub busy: Option<Busy>,
    pub toast: Option<(String, Tone, Instant)>,
    pub tick: usize,
    /// Directory the caller's shell should end up in (Open, or away from a removed worktree).
    pub cd_on_exit: Option<PathBuf>,
    pub quit: bool,
    pub shell_wrapper: bool,
    /// A newer release, once the background check finds one.
    pub update: Option<String>,
    /// Re-launch this (verified, updated) cmw after quitting.
    pub restart: Option<PathBuf>,
    /// Clickable areas of the last frame, topmost last.
    pub hits: RefCell<Vec<(Rect, Hit)>>,
    ctrl_c_armed: bool,
    tx: Sender<Msg>,
    size_cancel: Arc<AtomicBool>,
}

const TOAST_TTL: Duration = Duration::from_secs(4);

impl App {
    pub fn new(cwd: PathBuf, tx: Sender<Msg>) -> Self {
        let top = git::toplevel(&cwd).ok();
        let repo_name = top
            .as_ref()
            .and_then(|t| git::common_dir(t).ok())
            .map(|c| git::repo_name(&c));
        let branch = top
            .as_ref()
            .and_then(|t| git::run(t, &["branch", "--show-current"]).ok())
            .filter(|b| !b.is_empty());
        let default_base = top
            .as_ref()
            .and_then(|t| {
                let d = git::default_branch(t).ok()?;
                let remote = format!("refs/remotes/origin/{d}");
                Some(
                    if git::run_ok(t, &["show-ref", "--verify", "--quiet", &remote]) {
                        format!("origin/{d}")
                    } else {
                        d
                    },
                )
            })
            .unwrap_or_else(|| "default branch".into());
        let mut app = App {
            default_base,
            all: repo_name.is_none(),
            cwd,
            repo_name,
            branch,
            screen: Screen::Home,
            home_sel: 0,
            generation: 0,
            rows: Load::Loading,
            wt_sel: 0,
            filter: String::new(),
            filtering: false,
            gc: Load::Loading,
            gc_checked: Vec::new(),
            gc_sel: 0,
            slim_idle: 7,
            slim: Load::Loading,
            slim_sel: 0,
            new_branch: String::new(),
            new_base: String::new(),
            new_focus: 0,
            setup: Vec::new(),
            setup_sel: 0,
            modal: None,
            busy: None,
            toast: None,
            tick: 0,
            cd_on_exit: None,
            quit: false,
            shell_wrapper: std::env::var_os("CMW_CD_FILE").is_some(),
            update: None,
            restart: None,
            hits: RefCell::new(Vec::new()),
            ctrl_c_armed: false,
            tx,
            size_cancel: Arc::new(AtomicBool::new(false)),
        };
        app.refresh_setup();
        app
    }

    pub fn in_repo(&self) -> bool {
        self.repo_name.is_some()
    }

    pub fn menu(&self) -> Vec<&'static MenuItem> {
        MENU.iter()
            .filter(|m| m.screen != Screen::New || self.in_repo())
            .collect()
    }

    /// Reload everything shown (after startup, a scope switch, or a change on disk).
    pub fn reload(&mut self) {
        self.generation += 1;
        self.size_cancel.store(true, Ordering::Relaxed);
        self.size_cancel = Arc::new(AtomicBool::new(false));
        self.rows = Load::Loading;
        self.gc = Load::Loading;
        self.slim = Load::Loading;
        jobs::load_ls(&self.tx, &self.cwd, self.all, self.generation);
        jobs::load_gc_preview(&self.tx, &self.cwd, self.all, self.generation);
        jobs::load_slim_preview(
            &self.tx,
            &self.cwd,
            self.all,
            self.slim_idle,
            self.generation,
        );
    }

    pub fn on_tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        if self
            .toast
            .as_ref()
            .is_some_and(|(_, _, at)| at.elapsed() > TOAST_TTL)
        {
            self.toast = None;
        }
    }

    pub fn toast(&mut self, msg: impl Into<String>, tone: Tone) {
        self.toast = Some((msg.into(), tone, Instant::now()));
    }

    // -----------------------------------------------------------------------------------------
    // Messages from background jobs
    // -----------------------------------------------------------------------------------------

    /// Start the background release check (not part of `new`, so tests stay offline).
    pub fn check_update(&self) {
        jobs::check_update(&self.tx);
    }

    pub fn on_msg(&mut self, msg: Msg) {
        match msg {
            Msg::UpdateAvailable(v) => self.update = v,
            Msg::Ls { generation, res } if generation == self.generation => match res {
                Ok(report) => {
                    let rows = self.build_rows(report);
                    let paths = rows
                        .iter()
                        .filter(|r| r.usable())
                        .map(|r| r.st.path.clone())
                        .collect();
                    jobs::load_sizes(&self.tx, paths, generation, self.size_cancel.clone());
                    self.rows = Load::Ready(rows);
                    self.clamp_wt_sel();
                }
                Err(e) => self.rows = Load::Failed(e.message),
            },
            Msg::Size {
                generation,
                path,
                bytes,
            } if generation == self.generation => {
                if let Load::Ready(rows) = &mut self.rows
                    && let Some(r) = rows.iter_mut().find(|r| r.st.path == path)
                {
                    r.size = Some(bytes);
                }
            }
            Msg::GcPreview { generation, res } if generation == self.generation => {
                self.gc = match res {
                    Ok(r) => {
                        self.gc_checked = vec![true; r.would_remove.len()];
                        self.gc_sel = self.gc_sel.min(r.would_remove.len().saturating_sub(1));
                        Load::Ready(r)
                    }
                    Err(e) => Load::Failed(e.message),
                };
            }
            Msg::SlimPreview {
                generation,
                idle,
                res,
            } if generation == self.generation && idle == self.slim_idle => {
                self.slim = match res {
                    Ok(r) => {
                        self.slim_sel = self.slim_sel.min(r.worktrees.len().saturating_sub(1));
                        Load::Ready(r)
                    }
                    Err(e) => Load::Failed(e.message),
                };
            }
            Msg::Progress(line) => {
                if let Some(b) = &mut self.busy {
                    b.lines.extend(line.lines().map(str::to_string));
                    if b.lines.len() > 200 {
                        b.lines.drain(..100);
                    }
                }
            }
            Msg::Done(done) => {
                self.busy = None;
                self.ctrl_c_armed = false;
                self.on_done(done);
            }
            // Results of a superseded load.
            _ => {}
        }
    }

    fn build_rows(&self, report: crate::inventory::LsReport) -> Vec<Row> {
        // `here` is the deepest worktree containing cwd (worktrees can nest inside the main checkout).
        let here = report
            .repos
            .iter()
            .flat_map(|r| &r.worktrees)
            .filter(|w| self.cwd.starts_with(&w.path))
            .map(|w| w.path.clone())
            .max_by_key(|p| p.components().count());
        let mut rows = Vec::new();
        for repo in report.repos {
            let main = repo
                .worktrees
                .iter()
                .find(|w| w.is_main && !w.bare)
                .map(|w| w.path.clone());
            for st in repo.worktrees {
                rows.push(Row {
                    here: here.as_ref() == Some(&st.path),
                    repo: repo.repo.clone(),
                    default_branch: repo.default_branch.clone(),
                    repo_main: main.clone(),
                    st,
                    size: None,
                });
            }
        }
        rows
    }

    fn on_done(&mut self, done: Done) {
        match done {
            Done::New { res, warnings } => match res {
                Ok(r) => {
                    let mut text = r.path.display().to_string();
                    if let Some(h) = &r.hydrate {
                        text.push_str("\n\n");
                        text.push_str(&crate::output::Report::human(h));
                    }
                    let title = if r.created {
                        format!("Created {}", r.branch)
                    } else {
                        format!("{} already has a worktree", r.branch)
                    };
                    self.modal = Some(Modal::Result {
                        title,
                        text: jobs::with_warnings(text, &warnings),
                        tone: Tone::Ok,
                        open: Some(r.path),
                        scroll: 0,
                    });
                    self.new_branch.clear();
                    self.new_base.clear();
                    self.new_focus = 0;
                    self.screen = Screen::Worktrees;
                    self.reload();
                }
                Err(e) => self.show_error("Could not create the worktree", &e),
            },
            Done::Remove {
                target,
                force,
                res,
                warnings,
            } => {
                match res {
                    Ok(r) => {
                        if self.cwd.starts_with(&target.path) {
                            // We (and the caller's shell) were inside it: move to the main
                            // checkout so reloads, New and Clean keep working.
                            self.relocate(target.repo_main.clone());
                        }
                        let mut msg = format!("Removed {}", target.label());
                        if r.branch_deleted {
                            msg.push_str(" and its branch");
                        }
                        if warnings.is_empty() {
                            self.toast(msg, Tone::Ok);
                        } else {
                            self.modal = Some(Modal::Result {
                                title: msg,
                                text: jobs::with_warnings(String::new(), &warnings),
                                tone: Tone::Warn,
                                open: None,
                                scroll: 0,
                            });
                        }
                    }
                    Err(e) if e.kind == ErrorKind::Refused && !force && !is_unforceable(&e) => {
                        let expect = target.label();
                        self.modal = Some(Modal::TypeToConfirm {
                            title: format!("Remove {expect} anyway?"),
                            lines: vec![
                                e.message.clone(),
                                "Forcing it permanently deletes that work.".into(),
                            ],
                            expect,
                            input: String::new(),
                            action: Action::Remove {
                                target,
                                force: true,
                            },
                        });
                    }
                    Err(e) => self.show_error(&format!("Could not remove {}", target.label()), &e),
                }
                self.reload();
            }
            Done::Update { res, warnings } => match res {
                Ok(r) if r.updated => {
                    self.restart = r.installed_path.clone();
                    self.quit = true;
                }
                Ok(r) => {
                    self.update = None;
                    let mut msg = crate::output::Report::human(&r);
                    if !warnings.is_empty() {
                        msg = jobs::with_warnings(msg, &warnings);
                    }
                    self.toast(msg.lines().next().unwrap_or_default(), Tone::Ok);
                }
                Err(e) => self.show_error("Update failed", &e),
            },
            Done::Text { title, res } => {
                match res {
                    Ok(text) => {
                        let tone = if text.contains("warning:") {
                            Tone::Warn
                        } else {
                            Tone::Ok
                        };
                        self.modal = Some(Modal::Result {
                            title,
                            text,
                            tone,
                            open: None,
                            scroll: 0,
                        });
                    }
                    Err(e) => self.show_error("Failed", &e),
                }
                self.reload();
            }
        }
    }

    /// Continue from `dir` after the worktree we started in was removed.
    fn relocate(&mut self, dir: Option<PathBuf>) {
        let Some(dir) = dir.filter(|d| d.is_dir()) else {
            self.toast(
                "The worktree cmw started in was removed; restart cmw from another directory",
                Tone::Warn,
            );
            return;
        };
        self.branch = git::run(&dir, &["branch", "--show-current"])
            .ok()
            .filter(|b| !b.is_empty());
        self.cwd = dir.clone();
        self.cd_on_exit = Some(dir);
    }

    fn show_error(&mut self, title: &str, e: &CmwError) {
        let mut text = e.message.clone();
        if let Some(h) = &e.hint {
            text.push_str(&format!("\n\nfix: {h}"));
        }
        self.modal = Some(Modal::Result {
            title: title.to_string(),
            text,
            tone: Tone::Error,
            open: None,
            scroll: 0,
        });
    }

    // -----------------------------------------------------------------------------------------
    // Keys
    // -----------------------------------------------------------------------------------------

    pub fn on_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            if self.busy.is_none() || self.ctrl_c_armed {
                self.quit = true;
            } else {
                self.ctrl_c_armed = true;
                self.toast(
                    "A task is running. Press Ctrl-C again to quit anyway.",
                    Tone::Warn,
                );
            }
            return;
        }
        if self.busy.is_some() {
            return;
        }
        if self.modal.is_some() {
            self.on_modal_key(key);
            return;
        }
        let typing =
            self.screen == Screen::New || (self.screen == Screen::Worktrees && self.filtering);
        if typing {
            match self.screen {
                Screen::New => self.on_new_key(key),
                _ => self.on_filter_key(key),
            }
            return;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => {
                self.quit = true;
                return;
            }
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Left
                if self.screen != Screen::Home && self.screen != Screen::Slim =>
            {
                self.filter.clear();
                self.screen = Screen::Home;
                return;
            }
            KeyCode::Esc | KeyCode::Backspace if self.screen == Screen::Slim => {
                self.screen = Screen::Home;
                return;
            }
            KeyCode::Char('a') | KeyCode::Char('A') if self.in_repo() => {
                self.all = !self.all;
                self.wt_sel = 0;
                self.reload();
                let scope = if self.all {
                    "all repositories".to_string()
                } else {
                    self.repo_name.clone().unwrap_or_default()
                };
                self.toast(format!("Showing {scope}"), Tone::Ok);
                return;
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                self.reload();
                return;
            }
            KeyCode::Char('u') | KeyCode::Char('U') if self.update.is_some() => {
                self.ask_update();
                return;
            }
            _ => {}
        }
        match self.screen {
            Screen::Home => self.on_home_key(key),
            Screen::Worktrees => self.on_worktrees_key(key),
            Screen::Clean => self.on_clean_key(key),
            Screen::Slim => self.on_slim_key(key),
            Screen::Setup => self.on_setup_key(key),
            Screen::New => {}
        }
    }

    fn ask_update(&mut self) {
        let Some(latest) = self.update.clone() else {
            return;
        };
        let method = crate::update::detect();
        match method.command() {
            Some(cmd) => {
                self.modal = Some(Modal::Confirm {
                    title: format!("Update cmw to {latest}?"),
                    lines: vec![
                        format!("Runs {cmd}"),
                        String::new(),
                        "cmw restarts on the new version when it's done.".into(),
                    ],
                    danger: false,
                    action: Action::Update,
                });
            }
            None => {
                self.modal = Some(Modal::Result {
                    title: format!("cmw {latest} is available"),
                    text: format!("{}.\n\nfix: {}", method.refusal(), method.manual_hint()),
                    tone: Tone::Warn,
                    open: None,
                    scroll: 0,
                });
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // Mouse
    // -----------------------------------------------------------------------------------------

    pub fn on_mouse(&mut self, m: MouseEvent) {
        if self.busy.is_some() {
            return;
        }
        let press =
            |app: &mut App, code: KeyCode| app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
        match m.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let up = m.kind == MouseEventKind::ScrollUp;
                let code = if up { KeyCode::Up } else { KeyCode::Down };
                let key = KeyEvent::new(code, KeyModifiers::NONE);
                match &self.modal {
                    Some(Modal::Result { .. }) => press(self, code),
                    Some(_) => {}
                    // Typing owns the keyboard: scroll the list without leaving the filter.
                    None if self.screen == Screen::Worktrees && self.filtering => {
                        self.on_worktrees_key(key)
                    }
                    None if self.screen == Screen::New => {}
                    None => press(self, code),
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let hit = self
                    .hits
                    .borrow()
                    .iter()
                    .rev()
                    .find(|(r, _)| {
                        m.column >= r.x
                            && m.column < r.x + r.width
                            && m.row >= r.y
                            && m.row < r.y + r.height
                    })
                    .map(|(_, h)| *h);
                if let Some(hit) = hit {
                    self.on_hit(hit);
                }
            }
            _ => {}
        }
    }

    fn on_hit(&mut self, hit: Hit) {
        let press =
            |app: &mut App, code: KeyCode| app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
        match hit {
            Hit::Key(code) => press(self, code),
            Hit::UpdatePill => self.ask_update(),
            Hit::Menu(i) => {
                let menu = self.menu();
                if let Some(item) = menu.get(i) {
                    self.home_sel = i;
                    self.enter(item.screen);
                }
            }
            // Click selects; clicking the selected row activates it.
            Hit::Row(i) => match self.screen {
                Screen::Worktrees => {
                    self.filtering = false;
                    if self.wt_sel == i {
                        self.open_selected();
                    } else {
                        self.wt_sel = i;
                    }
                }
                Screen::Clean => {
                    if self.gc_sel == i {
                        press(self, KeyCode::Char(' '));
                    } else {
                        self.gc_sel = i;
                    }
                }
                Screen::Slim => self.slim_sel = i,
                Screen::Setup => {
                    if self.setup_sel == i {
                        press(self, KeyCode::Enter);
                    } else {
                        self.setup_sel = i;
                    }
                }
                Screen::New => {
                    self.new_focus = i.min(1);
                }
                Screen::Home => {}
            },
        }
    }

    fn enter(&mut self, screen: Screen) {
        self.screen = screen;
        if screen == Screen::Setup {
            self.refresh_setup();
        }
    }

    fn on_home_key(&mut self, key: KeyEvent) {
        let menu = self.menu();
        let n = menu.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.home_sel = (self.home_sel + n - 1) % n,
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                self.home_sel = (self.home_sel + 1) % n
            }
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                let screen = menu[self.home_sel.min(n - 1)].screen;
                self.enter(screen);
            }
            KeyCode::Char(c @ '1'..='9') => {
                let i = c as usize - '1' as usize;
                if i < n {
                    self.home_sel = i;
                    self.enter(menu[i].screen);
                }
            }
            _ => {}
        }
    }

    /// Rows matching the filter, as indices into the loaded rows.
    pub fn visible(&self) -> Vec<usize> {
        let Some(rows) = self.rows.ready() else {
            return Vec::new();
        };
        let f = self.filter.to_lowercase();
        rows.iter()
            .enumerate()
            .filter(|(_, r)| {
                f.is_empty()
                    || r.branch_label().to_lowercase().contains(&f)
                    || r.repo.to_lowercase().contains(&f)
                    || r.st.path.to_string_lossy().to_lowercase().contains(&f)
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Apparent size of a worktree, once measured.
    pub fn size_of(&self, path: &Path) -> Option<u64> {
        self.rows
            .ready()?
            .iter()
            .find(|r| r.st.path == path)
            .and_then(|r| r.size)
    }

    pub fn selected_row(&self) -> Option<&Row> {
        let vis = self.visible();
        let i = *vis.get(self.wt_sel)?;
        self.rows.ready()?.get(i)
    }

    fn clamp_wt_sel(&mut self) {
        let n = self.visible().len();
        self.wt_sel = self.wt_sel.min(n.saturating_sub(1));
    }

    fn on_filter_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.filter.clear();
                self.filtering = false;
            }
            KeyCode::Enter | KeyCode::Down | KeyCode::Up => {
                self.filtering = false;
                if key.code != KeyCode::Enter {
                    self.on_worktrees_key(key);
                }
            }
            KeyCode::Backspace => {
                if self.filter.pop().is_none() {
                    self.filtering = false;
                }
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.filter.clear()
            }
            KeyCode::Char(c) => self.filter.push(c),
            _ => {}
        }
        self.clamp_wt_sel();
    }

    fn on_worktrees_key(&mut self, key: KeyEvent) {
        let n = self.visible().len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.wt_sel = self.wt_sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.wt_sel = (self.wt_sel + 1).min(n.saturating_sub(1))
            }
            KeyCode::PageUp => self.wt_sel = self.wt_sel.saturating_sub(10),
            KeyCode::PageDown => self.wt_sel = (self.wt_sel + 10).min(n.saturating_sub(1)),
            KeyCode::Home | KeyCode::Char('g') => self.wt_sel = 0,
            KeyCode::End | KeyCode::Char('G') => self.wt_sel = n.saturating_sub(1),
            KeyCode::Char('/') => self.filtering = true,
            KeyCode::Enter => self.open_selected(),
            KeyCode::Char('h') | KeyCode::Char('H') => self.act_on_selected(Op::Hydrate),
            KeyCode::Char('e') | KeyCode::Char('E') => self.act_on_selected(Op::Env),
            KeyCode::Char('d') | KeyCode::Char('D') | KeyCode::Delete => {
                self.act_on_selected(Op::Remove)
            }
            _ => {}
        }
    }

    fn open_selected(&mut self) {
        let Some(row) = self.selected_row() else {
            return;
        };
        if !row.usable() {
            self.toast("That worktree's directory is missing", Tone::Error);
            return;
        }
        self.cd_on_exit = Some(row.st.path.clone());
        self.quit = true;
    }

    fn act_on_selected(&mut self, op: Op) {
        let Some(row) = self.selected_row() else {
            return;
        };
        if !row.usable() {
            self.toast("That worktree's directory is missing", Tone::Error);
            return;
        }
        let target = row.target();
        let label = target.label();
        match op {
            Op::Hydrate if row.st.is_main => self.toast(
                "The main checkout is the source; nothing to hydrate",
                Tone::Warn,
            ),
            Op::Env if row.st.is_main => self.toast(
                "The main checkout is the source; nothing to sync",
                Tone::Warn,
            ),
            Op::Remove if row.st.is_main => {
                self.toast("cmw never removes the main checkout", Tone::Warn)
            }
            Op::Hydrate => {
                self.start_busy(format!("Hydrating {label}"));
                jobs::hydrate(&self.tx, target);
            }
            Op::Env => {
                self.start_busy(format!("Syncing env for {label}"));
                jobs::env_sync(&self.tx, target);
            }
            Op::Remove => {
                let mut lines = vec![target.path.display().to_string()];
                lines.push(if row.st.merged == Some(true) {
                    "Deletes the directory, its build caches, and the merged branch.".into()
                } else {
                    "Deletes the directory and its build caches. The branch is kept.".into()
                });
                self.modal = Some(Modal::Confirm {
                    title: format!("Remove {label}?"),
                    lines,
                    danger: true,
                    action: Action::Remove {
                        target,
                        force: false,
                    },
                });
            }
        }
    }

    fn on_clean_key(&mut self, key: KeyEvent) {
        let Some(report) = self.gc.ready() else {
            return;
        };
        let n = report.would_remove.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.gc_sel = self.gc_sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.gc_sel = (self.gc_sel + 1).min(n.saturating_sub(1))
            }
            KeyCode::Char(' ') if n > 0 => {
                if let Some(c) = self.gc_checked.get_mut(self.gc_sel) {
                    *c = !*c;
                }
            }
            KeyCode::Enter => {
                let picked: Vec<Target> = report
                    .would_remove
                    .iter()
                    .zip(&self.gc_checked)
                    .filter(|(_, c)| **c)
                    .map(|(i, _)| Target {
                        path: i.path.clone(),
                        branch: i.branch.clone(),
                        repo_main: None,
                    })
                    .collect();
                if picked.is_empty() {
                    self.toast("Nothing selected", Tone::Warn);
                    return;
                }
                let mut lines: Vec<String> = picked
                    .iter()
                    .take(8)
                    .map(|t| format!("• {}", t.label()))
                    .collect();
                if picked.len() > 8 {
                    lines.push(format!("  and {} more", picked.len() - 8));
                }
                lines.push(String::new());
                lines.push(
                    "Directories, build caches and merged branches are deleted. Commits stay in the default branch."
                        .into(),
                );
                self.modal = Some(Modal::Confirm {
                    title: format!(
                        "Remove {} merged worktree{}?",
                        picked.len(),
                        plural(picked.len())
                    ),
                    lines,
                    danger: true,
                    action: Action::Clean { picked },
                });
            }
            _ => {}
        }
    }

    fn on_slim_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Left | KeyCode::Right | KeyCode::Char('h') | KeyCode::Char('l') => {
                let i = IDLE_STEPS
                    .iter()
                    .position(|d| *d == self.slim_idle)
                    .unwrap_or(3);
                let i = if matches!(key.code, KeyCode::Left | KeyCode::Char('h')) {
                    i.saturating_sub(1)
                } else {
                    (i + 1).min(IDLE_STEPS.len() - 1)
                };
                if IDLE_STEPS[i] != self.slim_idle {
                    self.slim_idle = IDLE_STEPS[i];
                    self.slim = Load::Loading;
                    jobs::load_slim_preview(
                        &self.tx,
                        &self.cwd,
                        self.all,
                        self.slim_idle,
                        self.generation,
                    );
                }
            }
            KeyCode::Up | KeyCode::Char('k') => self.slim_sel = self.slim_sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                let n = self.slim.ready().map_or(0, |r| r.worktrees.len());
                self.slim_sel = (self.slim_sel + 1).min(n.saturating_sub(1));
            }
            KeyCode::Enter => {
                let Some(r) = self.slim.ready() else {
                    return;
                };
                if r.worktrees.is_empty() {
                    self.toast("Nothing to delete", Tone::Warn);
                    return;
                }
                let n = r.worktrees.len();
                self.modal = Some(Modal::Confirm {
                    title: format!(
                        "Delete {} of build caches?",
                        human_bytes(r.total_apparent_bytes)
                    ),
                    lines: vec![
                        format!(
                            "From {n} worktree{} idle for {}.",
                            plural(n),
                            idle_label(self.slim_idle)
                        ),
                        "Everything deleted is regenerated by the next build.".into(),
                    ],
                    danger: false,
                    action: Action::Slim {
                        idle: self.slim_idle,
                    },
                });
            }
            _ => {}
        }
    }

    fn on_new_key(&mut self, key: KeyEvent) {
        let field = if self.new_focus == 0 {
            &mut self.new_branch
        } else {
            &mut self.new_base
        };
        match key.code {
            KeyCode::Esc => self.screen = Screen::Home,
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down => {
                self.new_focus = 1 - self.new_focus
            }
            KeyCode::Backspace => {
                field.pop();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => field.clear(),
            KeyCode::Char(c) if !c.is_whitespace() => field.push(c),
            KeyCode::Enter => {
                let branch = self.new_branch.trim().to_string();
                if branch.is_empty() {
                    self.new_focus = 0;
                    self.toast("Type a branch name", Tone::Warn);
                    return;
                }
                let base = Some(self.new_base.trim().to_string()).filter(|b| !b.is_empty());
                self.start_busy(format!("Creating {branch}"));
                jobs::new_worktree(&self.tx, &self.cwd, branch, base);
            }
            _ => {}
        }
    }

    pub fn refresh_setup(&mut self) {
        let shell = detect_shell();
        let shell_row = if self.shell_wrapper {
            SetupRow {
                title: "Shell integration",
                status: "active".into(),
                tone: Some(Tone::Ok),
            }
        } else {
            match &shell {
                Some(s) if rc_has_init(&s.rc) => SetupRow {
                    title: "Shell integration",
                    status: "added; open a new shell".into(),
                    tone: Some(Tone::Warn),
                },
                Some(s) => SetupRow {
                    title: "Shell integration",
                    status: format!("not set up ({})", s.name),
                    tone: Some(Tone::Error),
                },
                None => SetupRow {
                    title: "Shell integration",
                    status: "unsupported shell".into(),
                    tone: Some(Tone::Error),
                },
            }
        };
        let st = crate::agents::status();
        let (skills, skills_tone) = skills_summary(&st);
        let hooks = st.hooks();
        self.setup = vec![
            shell_row,
            SetupRow {
                title: "Agent skills",
                status: skills,
                tone: Some(skills_tone),
            },
            SetupRow {
                title: "Claude Code hooks",
                status: hooks.into(),
                // Optional, so "off" is an action rather than a to-do.
                tone: match hooks {
                    "on" => Some(Tone::Ok),
                    "partial" => Some(Tone::Warn),
                    _ => None,
                },
            },
            SetupRow {
                title: "Agent setup prompt",
                status: "copy".into(),
                tone: None,
            },
            SetupRow {
                title: "AGENTS.md snippet",
                status: "copy".into(),
                tone: None,
            },
        ];
    }

    fn on_setup_key(&mut self, key: KeyEvent) {
        let n = self.setup.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.setup_sel = self.setup_sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.setup_sel = (self.setup_sel + 1).min(n.saturating_sub(1))
            }
            KeyCode::Enter => match self.setup_sel {
                0 => {
                    if self.shell_wrapper {
                        self.toast("Shell integration is active", Tone::Ok);
                        return;
                    }
                    let Some(s) = detect_shell() else {
                        self.toast("Supported shells: zsh, bash, fish", Tone::Warn);
                        return;
                    };
                    if rc_has_init(&s.rc) {
                        self.toast("Already added. Open a new shell to use it.", Tone::Ok);
                        return;
                    }
                    self.modal = Some(Modal::Confirm {
                        title: format!("Add cmw to {}?", tilde(&s.rc)),
                        lines: vec![
                            s.line.clone(),
                            String::new(),
                            "Adds `cmw cd <name>`, and Open in this screen changes your directory."
                                .into(),
                        ],
                        danger: false,
                        action: Action::InstallShell {
                            rc: s.rc,
                            line: s.line,
                        },
                    });
                }
                1 => {
                    let args = crate::cli::AgentsInstallArgs::default();
                    match crate::agents::install(&Ctx::new(true, "agents install"), &args) {
                        Ok(r) => self.toast(r.summary(), Tone::Ok),
                        Err(e) => self.toast(e.message, Tone::Error),
                    }
                    self.refresh_setup();
                }
                2 => {
                    if crate::agents::status().hooks() == "on" {
                        self.toast("Claude Code hooks are on", Tone::Ok);
                        return;
                    }
                    self.modal = Some(Modal::Confirm {
                        title: "Route Claude Code worktrees through cmw?".into(),
                        lines: vec![
                            "Adds WorktreeCreate and WorktreeRemove hooks to ~/.claude/settings.json."
                                .into(),
                            String::new(),
                            "claude --worktree then creates worktrees in ~/.cmw/worktrees and keeps any with unsaved work on removal."
                                .into(),
                        ],
                        danger: false,
                        action: Action::InstallClaudeHooks,
                    });
                }
                3 => self.copy(
                    crate::agents::PROMPT_MD,
                    "Prompt copied. Paste it into Claude Code, Codex, Grok or T3 Code",
                ),
                _ => self.copy(&skill::print(true).text, "AGENTS.md snippet copied"),
            },
            _ => {}
        }
    }

    fn copy(&mut self, text: &str, done: &str) {
        if crate::clipboard::copy(text) {
            self.toast(done, Tone::Ok);
        } else {
            self.toast("Could not reach the clipboard (pbcopy)", Tone::Error);
        }
    }

    fn on_modal_key(&mut self, key: KeyEvent) {
        let Some(modal) = self.modal.take() else {
            return;
        };
        match modal {
            Modal::Confirm {
                title,
                lines,
                danger,
                action,
            } => match key.code {
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => self.run_action(action),
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Char('q') => {}
                _ => {
                    self.modal = Some(Modal::Confirm {
                        title,
                        lines,
                        danger,
                        action,
                    })
                }
            },
            Modal::TypeToConfirm {
                title,
                lines,
                expect,
                mut input,
                action,
            } => match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter if input == expect => self.run_action(action),
                code => {
                    match code {
                        KeyCode::Backspace => {
                            input.pop();
                        }
                        KeyCode::Char(c) => input.push(c),
                        _ => {}
                    }
                    self.modal = Some(Modal::TypeToConfirm {
                        title,
                        lines,
                        expect,
                        input,
                        action,
                    });
                }
            },
            Modal::Result {
                title,
                text,
                tone,
                open,
                scroll,
            } => match key.code {
                KeyCode::Enter if open.is_some() => {
                    self.cd_on_exit = open;
                    self.quit = true;
                }
                KeyCode::Up | KeyCode::Down | KeyCode::Char('k') | KeyCode::Char('j') => {
                    let scroll = if matches!(key.code, KeyCode::Up | KeyCode::Char('k')) {
                        scroll.saturating_sub(1)
                    } else {
                        scroll.saturating_add(1).min(text.lines().count() as u16)
                    };
                    self.modal = Some(Modal::Result {
                        title,
                        text,
                        tone,
                        open,
                        scroll,
                    });
                }
                _ => {}
            },
        }
    }

    fn run_action(&mut self, action: Action) {
        match action {
            Action::Remove { target, force } => {
                self.start_busy(format!("Removing {}", target.label()));
                jobs::remove(&self.tx, target, force);
            }
            Action::Clean { picked } => {
                self.start_busy("Removing merged worktrees".into());
                jobs::clean(&self.tx, picked);
            }
            Action::Slim { idle } => {
                self.start_busy("Deleting build caches".into());
                jobs::slim(&self.tx, &self.cwd, self.all, idle);
            }
            Action::Update => {
                let to = self.update.clone().unwrap_or_default();
                self.start_busy(format!("Updating cmw to {to}"));
                jobs::run_update(&self.tx);
            }
            Action::InstallClaudeHooks => {
                match crate::agents::install_hooks(&self.cwd, false) {
                    Ok(r) => self.toast(
                        format!(
                            "Claude Code hooks {} in {}",
                            r.status,
                            tilde(Path::new(&r.path))
                        ),
                        Tone::Ok,
                    ),
                    Err(e) => self.toast(e.message, Tone::Error),
                }
                self.refresh_setup();
            }
            Action::InstallShell { rc, line } => {
                match append_line(&rc, &line) {
                    Ok(()) => self.toast(
                        format!("Added to {}. Open a new shell to use it.", tilde(&rc)),
                        Tone::Ok,
                    ),
                    Err(e) => {
                        self.toast(format!("Could not write {}: {e}", tilde(&rc)), Tone::Error)
                    }
                }
                self.refresh_setup();
            }
        }
    }

    fn start_busy(&mut self, label: String) {
        self.busy = Some(Busy {
            label,
            started: Instant::now(),
            lines: Vec::new(),
        });
    }
}

enum Op {
    Hydrate,
    Env,
    Remove,
}

/// Refusals that `--force` does not override.
fn is_unforceable(e: &CmwError) -> bool {
    e.message.contains("main checkout") || e.message.contains("is locked")
}

pub fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

pub fn idle_label(days: u64) -> String {
    match days {
        0 => "any time".into(),
        1 => "1+ day".into(),
        d => format!("{d}+ days"),
    }
}

pub use crate::paths::tilde;

/// Setup-screen status for the cmw skill across detected agents (all of them when none detected).
fn skills_summary(st: &crate::agents::StatusReport) -> (String, Tone) {
    let mut relevant: Vec<_> = st.detected_with_skill().collect();
    if relevant.is_empty() {
        relevant = st.agents.iter().filter(|a| a.skill.is_some()).collect();
    }
    let state = |a: &crate::agents::AgentStatus| a.skill.as_ref().map_or("missing", |s| s.status);
    let names = |want: &str| {
        relevant
            .iter()
            .filter(|a| state(a) == want)
            .map(|a| a.name)
            .collect::<Vec<_>>()
            .join(", ")
    };
    if relevant.iter().all(|a| state(a) == "missing") {
        ("not installed".into(), Tone::Error)
    } else if !names("outdated").is_empty() {
        ("outdated".into(), Tone::Warn)
    } else if relevant.iter().any(|a| state(a) == "missing") {
        (format!("missing for {}", names("missing")), Tone::Warn)
    } else {
        // Edited skills count as installed: `cmw update` leaves them alone on purpose.
        let present: Vec<_> = relevant.iter().map(|a| a.name).collect();
        (format!("installed ({})", present.join(", ")), Tone::Ok)
    }
}

struct ShellInfo {
    name: &'static str,
    rc: PathBuf,
    line: String,
}

fn detect_shell() -> Option<ShellInfo> {
    let shell = std::env::var("SHELL").ok()?;
    let home = crate::paths::home();
    match Path::new(&shell).file_name()?.to_str()? {
        "zsh" => {
            let dir = std::env::var_os("ZDOTDIR").map_or(home, PathBuf::from);
            Some(ShellInfo {
                name: "zsh",
                rc: dir.join(".zshrc"),
                line: r#"eval "$(cmw shell-init zsh)""#.into(),
            })
        }
        "bash" => {
            // macOS terminals start login shells, which read .bash_profile.
            let profile = home.join(".bash_profile");
            let rc = if profile.exists() {
                profile
            } else {
                home.join(".bashrc")
            };
            Some(ShellInfo {
                name: "bash",
                rc,
                line: r#"eval "$(cmw shell-init bash)""#.into(),
            })
        }
        "fish" => Some(ShellInfo {
            name: "fish",
            rc: home.join(".config/fish/config.fish"),
            line: "cmw shell-init fish | source".into(),
        }),
        _ => None,
    }
}

fn rc_has_init(rc: &Path) -> bool {
    std::fs::read_to_string(rc).is_ok_and(|s| s.contains("cmw shell-init"))
}

fn append_line(rc: &Path, line: &str) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = rc.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let existing = std::fs::read_to_string(rc).unwrap_or_default();
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(rc)?;
    let sep = if existing.is_empty() || existing.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    writeln!(
        f,
        "{sep}\n# cmw: `cmw cd <name>` and cd from the TUI\n{line}"
    )
}
