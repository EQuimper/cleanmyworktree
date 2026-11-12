//! Rendering. Pure functions of `App` + `Theme`, so screens are testable with `TestBackend`.
use super::app::{App, IDLE_STEPS, Load, Modal, Row, Screen, Tone, idle_label, plural, tilde};
use super::theme::Theme;
use crate::output::human_bytes;
use crate::{git, paths};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};

const LOGO: [&str; 5] = [
    " ██████ ███    ███ ██     ██",
    "██      ████  ████ ██     ██",
    "██      ██ ████ ██ ██  █  ██",
    "██      ██  ██  ██ ██ ███ ██",
    " ██████ ██      ██  ███ ███ ",
];
const LOGO_W: u16 = 28;
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const PAD: u16 = 2;

pub fn render(f: &mut Frame, app: &App, t: &Theme) {
    let area = f.area();
    if area.width < 44 || area.height < 14 {
        let msg = Paragraph::new("Make the terminal a little bigger")
            .style(t.fg(t.dim))
            .alignment(Alignment::Center);
        f.render_widget(msg, center(area, area.width, 1));
        return;
    }
    let [main, toast] = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(area);
    match app.screen {
        Screen::Home => home(f, main, app, t),
        screen => page(f, main, app, t, screen),
    }
    render_toast(f, toast, app, t);
    if let Some(m) = &app.modal {
        modal(f, area, m, t);
    }
    if app.busy.is_some() {
        busy(f, area, app, t);
    }
}

fn spinner(app: &App) -> &'static str {
    SPINNER[app.tick % SPINNER.len()]
}

fn inset(area: Rect) -> Rect {
    Rect {
        x: area.x + PAD,
        y: area.y + 1,
        width: area.width.saturating_sub(PAD * 2),
        height: area.height.saturating_sub(1),
    }
}

fn center(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

/// `"Key Label"` pairs as a Mole-style footer: `↑↓  |  Enter  |  Q Quit`.
fn footer<'a>(keys: &[(&'a str, &'a str)], t: &Theme) -> Line<'a> {
    let mut spans = Vec::new();
    for (i, (k, label)) in keys.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  |  ", t.fg(t.faint)));
        }
        spans.push(Span::styled(*k, t.fg(t.dim)));
        if !label.is_empty() {
            spans.push(Span::styled(format!(" {label}"), t.fg(t.dim)));
        }
    }
    Line::from(spans)
}

fn scope_keys(app: &App) -> Option<(&'static str, &'static str)> {
    app.in_repo()
        .then_some(("A", if app.all { "This repo" } else { "All repos" }))
}

fn truncate(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        s.to_string()
    } else if w <= 1 {
        "…".chars().take(w).collect()
    } else {
        let mut out: String = s.chars().take(w - 1).collect();
        out.push('…');
        out
    }
}

fn pad(s: &str, w: usize) -> String {
    let s = truncate(s, w);
    let n = s.chars().count();
    format!("{s}{}", " ".repeat(w.saturating_sub(n)))
}

fn rpad(s: &str, w: usize) -> String {
    let s = truncate(s, w);
    let n = s.chars().count();
    format!("{}{s}", " ".repeat(w.saturating_sub(n)))
}

// ---------------------------------------------------------------------------------------------
// Home
// ---------------------------------------------------------------------------------------------

fn home(f: &mut Frame, area: Rect, app: &App, t: &Theme) {
    let area = inset(area);
    let menu = app.menu();
    let show_logo = area.height as usize >= LOGO.len() + menu.len() + 6;
    let mut lines: Vec<Line> = Vec::new();

    let scope = if app.all {
        "All repositories".to_string()
    } else {
        app.repo_name.clone().unwrap_or_default()
    };
    let info: [Line; 5] = [
        Line::default(),
        Line::from(Span::styled(
            scope,
            Style::new().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            app.branch.clone().unwrap_or_default(),
            t.fg(t.dim),
        )),
        Line::from(Span::styled(
            concat!("v", env!("CARGO_PKG_VERSION")),
            t.fg(t.faint),
        )),
        Line::default(),
    ];
    if show_logo {
        let side = area.width >= LOGO_W + 4 + 20;
        for (i, row) in LOGO.iter().enumerate() {
            let mut spans = vec![Span::styled(*row, t.fg(t.gradient(i, LOGO.len())))];
            if side {
                spans.push(Span::raw("    "));
                spans.extend(info[i].spans.clone());
            }
            lines.push(Line::from(spans));
        }
        lines.push(Line::default());
    } else {
        lines.push(Line::from(vec![
            Span::styled("cmw", t.accent_bold()),
            Span::raw("  "),
            Span::styled(
                info[3]
                    .spans
                    .first()
                    .map(|s| s.content.to_string())
                    .unwrap_or_default(),
                Style::new().add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::default());
    }

    let label_w = menu.iter().map(|m| m.label.len()).max().unwrap_or(0) + 4;
    let desc_w = menu.iter().map(|m| m.desc.len()).max().unwrap_or(0) + 3;
    for (i, item) in menu.iter().enumerate() {
        let sel = i == app.home_sel;
        let (arrow, label_style, desc_style) = if sel {
            ("➤ ", t.accent_bold(), t.fg(t.accent))
        } else {
            ("  ", Style::new(), t.fg(t.dim))
        };
        let mut spans = vec![
            Span::styled(arrow, t.fg(t.accent)),
            Span::styled(
                format!("{}. ", i + 1),
                if sel { t.fg(t.accent) } else { t.fg(t.faint) },
            ),
            Span::styled(pad(item.label, label_w), label_style),
            Span::styled(pad(item.desc, desc_w), desc_style),
        ];
        if let Some((badge, tone)) = badge(app, item.screen) {
            spans.push(Span::styled(badge, tone_style(t, tone)));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::default());
    let mut keys = vec![("↑↓", ""), ("Enter", "")];
    keys.extend(scope_keys(app));
    keys.push(("Q", "Quit"));
    lines.push(footer(&keys, t));
    f.render_widget(Paragraph::new(lines), area);
}

/// Live counts shown at the end of a home menu row.
fn badge(app: &App, screen: Screen) -> Option<(String, Option<Tone>)> {
    let loading = || Some((spinner(app).to_string(), None));
    match screen {
        Screen::Worktrees => match &app.rows {
            Load::Loading => loading(),
            Load::Ready(rows) => {
                let linked = rows.iter().filter(|r| !r.st.is_main).count();
                Some((format!("{linked} linked"), None))
            }
            Load::Failed(_) => Some(("error".into(), Some(Tone::Error))),
        },
        Screen::Clean => match &app.gc {
            Load::Loading => loading(),
            Load::Ready(r) if r.would_remove.is_empty() => Some(("all clean".into(), None)),
            Load::Ready(r) => Some((format!("{} ready", r.would_remove.len()), Some(Tone::Ok))),
            Load::Failed(_) => Some(("error".into(), Some(Tone::Error))),
        },
        Screen::Slim => match &app.slim {
            Load::Loading => loading(),
            Load::Ready(r) if r.total_apparent_bytes == 0 => Some(("nothing idle".into(), None)),
            Load::Ready(r) => Some((human_bytes(r.total_apparent_bytes), Some(Tone::Ok))),
            Load::Failed(_) => Some(("error".into(), Some(Tone::Error))),
        },
        Screen::Setup => {
            let todo = app
                .setup
                .iter()
                .filter(|s| s.tone == Some(Tone::Error))
                .count();
            (todo > 0).then(|| (format!("{todo} to do"), Some(Tone::Warn)))
        }
        _ => None,
    }
}

fn tone_style(t: &Theme, tone: Option<Tone>) -> Style {
    match tone {
        None => t.fg(t.faint),
        Some(Tone::Ok) => t.fg(t.accent),
        Some(Tone::Warn) => t.fg(t.warn),
        Some(Tone::Error) => t.fg(t.danger),
    }
}

// ---------------------------------------------------------------------------------------------
// Sub-screens
// ---------------------------------------------------------------------------------------------

fn page(f: &mut Frame, area: Rect, app: &App, t: &Theme, screen: Screen) {
    let area = inset(area);
    let [head, rule, body, foot] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(2),
    ])
    .areas(area);

    let title = match screen {
        Screen::Worktrees => "Worktrees",
        Screen::New => "New worktree",
        Screen::Clean => "Clean",
        Screen::Slim => "Slim",
        Screen::Setup => "Setup",
        Screen::Home => "",
    };
    let scope = if app.all {
        "all repositories".to_string()
    } else {
        app.repo_name.clone().unwrap_or_default()
    };
    let mut spans = vec![
        Span::styled("cmw", t.accent_bold()),
        Span::styled(" › ", t.fg(t.faint)),
        Span::styled(title, Style::new().add_modifier(Modifier::BOLD)),
    ];
    if screen != Screen::Setup {
        spans.push(Span::styled(format!("   {scope}"), t.fg(t.dim)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), head);
    if screen == Screen::Worktrees && (app.filtering || !app.filter.is_empty()) {
        let cursor = if app.filtering { "▏" } else { "" };
        let filter = Line::from(vec![
            Span::styled("/ ", t.fg(t.accent)),
            Span::raw(format!("{}{cursor}", app.filter)),
        ]);
        f.render_widget(Paragraph::new(filter).alignment(Alignment::Right), head);
    }
    f.render_widget(
        Paragraph::new(Span::styled("─".repeat(rule.width as usize), t.fg(t.faint))),
        rule,
    );
    let body = Rect {
        y: body.y + 1,
        height: body.height.saturating_sub(1),
        ..body
    };

    let mut keys: Vec<(&str, &str)> = match screen {
        Screen::Worktrees if app.filtering => {
            vec![("Enter", "Done"), ("Esc", "Clear")]
        }
        Screen::Worktrees => vec![
            ("↑↓", ""),
            ("Enter", "Open"),
            ("H", "Hydrate"),
            ("E", "Env"),
            ("D", "Remove"),
            ("/", "Filter"),
        ],
        Screen::New => vec![("Tab", "Next field"), ("Enter", "Create"), ("Esc", "Back")],
        Screen::Clean => vec![("↑↓", ""), ("Space", "Toggle"), ("Enter", "Remove")],
        Screen::Slim => vec![("←→", "Idle days"), ("Enter", "Delete")],
        Screen::Setup => vec![("↑↓", ""), ("Enter", "")],
        Screen::Home => vec![],
    };
    if !matches!(screen, Screen::New | Screen::Setup) && !app.filtering {
        keys.extend(scope_keys(app));
    }
    if !matches!(screen, Screen::New) && !app.filtering {
        keys.push(("Esc", "Back"));
    }
    let foot = Rect {
        y: foot.y + 1,
        height: 1,
        ..foot
    };
    f.render_widget(Paragraph::new(footer(&keys, t)), foot);

    match screen {
        Screen::Worktrees => worktrees(f, body, app, t),
        Screen::New => new(f, body, app, t),
        Screen::Clean => clean(f, body, app, t),
        Screen::Slim => slim(f, body, app, t),
        Screen::Setup => setup(f, body, app, t),
        Screen::Home => {}
    }
}

fn loading_line<'a>(app: &App, t: &Theme, what: &'a str) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("{} ", spinner(app)), t.fg(t.accent)),
        Span::styled(what, t.fg(t.dim)),
    ])
}

fn failed_line<'a>(t: &Theme, msg: &str) -> Line<'a> {
    Line::from(Span::styled(format!("✗ {msg}"), t.fg(t.danger)))
}

/// The selected row, its background running the full width.
fn highlight<'a>(mut line: Line<'a>, width: u16, t: &Theme) -> Line<'a> {
    let fill = (width as usize).saturating_sub(line.width());
    line.spans.push(Span::raw(" ".repeat(fill)));
    line.style(t.selected())
}

/// First visible index so that `sel` stays on screen.
fn scroll_offset(sel: usize, height: usize) -> usize {
    if height == 0 || sel < height {
        0
    } else {
        sel + 1 - height
    }
}

fn state_cell(r: &Row, t: &Theme) -> (String, Style) {
    let st = &r.st;
    if st.bare {
        ("bare".into(), t.fg(t.faint))
    } else if st.prunable || !st.path.is_dir() {
        ("✗ missing".into(), t.fg(t.danger))
    } else if st.error.is_some() {
        ("! unreadable".into(), t.fg(t.danger))
    } else if st.dirty > 0 {
        (format!("● {} changed", st.dirty), t.fg(t.warn))
    } else if !st.is_main && st.merged == Some(true) {
        ("✓ merged".into(), t.fg(t.accent))
    } else if st.unpushed > 0 {
        (format!("↑ {} unpushed", st.unpushed), t.fg(t.info))
    } else {
        ("clean".into(), t.fg(t.dim))
    }
}

/// `merge_reason` codes from inventory, as prose.
fn merge_reason(reason: &str) -> &str {
    match reason {
        "no_commits" => "no commits of its own yet",
        "has_merge_commits" => "contains merge commits, so squash detection can't vouch for it",
        other => other,
    }
}

/// Absolute paths under $HOME shortened to `~/…` in free text (command reports, errors).
fn tildify(text: &str) -> String {
    let home = paths::home();
    let home = home.to_string_lossy();
    if home.len() <= 1 {
        return text.to_string();
    }
    text.replace(&format!("{home}/"), "~/")
}

/// Faint tags after the branch name.
fn suffix(r: &Row) -> String {
    let mut s = String::new();
    if r.st.is_main && r.st.branch.as_deref() != Some(r.default_branch.as_str()) {
        s.push_str("  main checkout");
    }
    if r.here {
        s.push_str("  ◂ here");
    }
    s
}

fn idle_cell(r: &Row) -> String {
    match r.st.idle_days {
        Some(0) => "today".into(),
        Some(d) => format!("{d}d"),
        None => "-".into(),
    }
}

fn worktrees(f: &mut Frame, area: Rect, app: &App, t: &Theme) {
    let rows = match &app.rows {
        Load::Loading => {
            f.render_widget(
                Paragraph::new(loading_line(app, t, "Reading worktrees…")),
                area,
            );
            return;
        }
        Load::Failed(e) => {
            f.render_widget(Paragraph::new(failed_line(t, e)), area);
            return;
        }
        Load::Ready(rows) => rows,
    };
    let vis = app.visible();
    if vis.is_empty() {
        let msg = if app.filter.is_empty() {
            "No worktrees"
        } else {
            "Nothing matches the filter"
        };
        f.render_widget(Paragraph::new(Span::styled(msg, t.fg(t.dim))), area);
        return;
    }

    let [table, detail] = Layout::vertical([Constraint::Min(0), Constraint::Length(2)]).areas(area);
    let w = table.width as usize;
    let (state_w, idle_w, size_w) = (14, 6, 9);
    let repo_w = if app.all {
        vis.iter()
            .map(|&i| rows[i].repo.chars().count())
            .max()
            .unwrap_or(4)
            .clamp(4, 22)
    } else {
        0
    };
    let fixed = 2 + state_w + idle_w + size_w + 6 + if app.all { repo_w + 2 } else { 0 };
    let content_w = vis
        .iter()
        .map(|&i| rows[i].branch_label().chars().count() + suffix(&rows[i]).chars().count())
        .max()
        .unwrap_or(10);
    let branch_w = content_w.clamp(10, w.saturating_sub(fixed).max(10));

    let mut lines = Vec::new();
    let mut head = String::from("  ");
    if app.all {
        head.push_str(&pad("REPO", repo_w + 2));
    }
    head.push_str(&pad("BRANCH", branch_w + 2));
    head.push_str(&pad("STATE", state_w + 2));
    head.push_str(&rpad("IDLE", idle_w));
    head.push_str(&rpad("SIZE", size_w + 2));
    lines.push(Line::from(Span::styled(head, t.fg(t.faint))));

    let height = (table.height as usize).saturating_sub(1);
    let offset = scroll_offset(app.wt_sel, height);
    for (pos, &i) in vis.iter().enumerate().skip(offset).take(height) {
        let r = &rows[i];
        let sel = pos == app.wt_sel;
        let mut spans = vec![Span::styled(if sel { "➤ " } else { "  " }, t.fg(t.accent))];
        if app.all {
            spans.push(Span::styled(pad(&r.repo, repo_w + 2), t.fg(t.dim)));
        }
        let suffix = suffix(r);
        let label = truncate(
            &r.branch_label(),
            branch_w.saturating_sub(suffix.chars().count()),
        );
        let used = label.chars().count() + suffix.chars().count();
        spans.push(Span::styled(
            label,
            if sel { t.accent_bold() } else { Style::new() },
        ));
        spans.push(Span::styled(suffix, t.fg(t.faint)));
        spans.push(Span::raw(" ".repeat((branch_w + 2).saturating_sub(used))));
        let (state, style) = state_cell(r, t);
        spans.push(Span::styled(pad(&state, state_w + 2), style));
        spans.push(Span::styled(rpad(&idle_cell(r), idle_w), t.fg(t.dim)));
        let size = match r.size {
            Some(b) => human_bytes(b),
            None if r.usable() => spinner(app).to_string(),
            None => "-".into(),
        };
        spans.push(Span::styled(rpad(&size, size_w + 2), t.fg(t.dim)));
        let mut line = Line::from(spans);
        if sel {
            line = highlight(line, table.width, t);
        }
        lines.push(line);
    }
    f.render_widget(Paragraph::new(lines), table);

    if let Some(r) = app.selected_row() {
        let mut d = vec![Line::from(Span::styled(
            format!("  {}", tilde(&r.st.path)),
            t.fg(t.dim),
        ))];
        if let Some(e) = &r.st.error {
            d.push(Line::from(Span::styled(format!("  {e}"), t.fg(t.danger))));
        } else if !r.st.merge_reason.is_empty() && !r.st.is_main {
            d.push(Line::from(Span::styled(
                format!("  {}", merge_reason(&r.st.merge_reason)),
                t.fg(t.faint),
            )));
        }
        f.render_widget(Paragraph::new(d), detail);
    }
}

fn field<'a>(label: &'a str, value: &str, placeholder: &str, focus: bool, t: &Theme) -> Line<'a> {
    let marker = if focus { "➤ " } else { "  " };
    let mut spans = vec![
        Span::styled(marker, t.fg(t.accent)),
        Span::styled(
            format!("{label:<8}"),
            if focus { t.accent_bold() } else { t.fg(t.dim) },
        ),
    ];
    if value.is_empty() {
        if focus {
            spans.push(Span::styled("▏", t.fg(t.accent)));
        }
        spans.push(Span::styled(placeholder.to_string(), t.fg(t.faint)));
    } else {
        spans.push(Span::raw(value.to_string()));
        if focus {
            spans.push(Span::styled("▏", t.fg(t.accent)));
        }
    }
    Line::from(spans)
}

fn new(f: &mut Frame, area: Rect, app: &App, t: &Theme) {
    let mut lines = vec![
        field(
            "Branch",
            &app.new_branch,
            "feat/my-change",
            app.new_focus == 0,
            t,
        ),
        Line::default(),
        field(
            "Base",
            &app.new_base,
            &app.default_base,
            app.new_focus == 1,
            t,
        ),
        Line::default(),
        Line::default(),
    ];
    let branch = app.new_branch.trim();
    if !branch.is_empty() {
        let dest = paths::worktrees_root()
            .join(app.repo_name.as_deref().unwrap_or("repo"))
            .join(git::slugify(branch));
        lines.push(Line::from(vec![
            Span::styled("  → ", t.fg(t.faint)),
            Span::styled(tilde(&dest), t.fg(t.dim)),
        ]));
    }
    f.render_widget(Paragraph::new(lines), area);
}

fn clean(f: &mut Frame, area: Rect, app: &App, t: &Theme) {
    let r = match &app.gc {
        Load::Loading => {
            f.render_widget(
                Paragraph::new(loading_line(app, t, "Checking what's merged…")),
                area,
            );
            return;
        }
        Load::Failed(e) => {
            f.render_widget(Paragraph::new(failed_line(t, e)), area);
            return;
        }
        Load::Ready(r) => r,
    };
    let mut lines = Vec::new();
    if r.would_remove.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("✓ ", t.fg(t.accent)),
            Span::raw("Nothing to clean. No merged worktrees."),
        ]));
    } else {
        let branch_w = r
            .would_remove
            .iter()
            .map(|i| i.branch.as_deref().unwrap_or("(detached)").chars().count())
            .max()
            .unwrap_or(10)
            .clamp(10, 40);
        let repo_w = if app.all {
            r.would_remove
                .iter()
                .map(|i| i.repo.chars().count())
                .max()
                .unwrap_or(4)
                .clamp(4, 22)
                + 2
        } else {
            0
        };
        let (via_w, size_w) = (12, 10);
        let path_w =
            (area.width as usize).saturating_sub(4 + repo_w + branch_w + 2 + via_w + size_w + 2);
        let mut head = format!("    {}", pad(if app.all { "REPO" } else { "" }, repo_w));
        head.push_str(&pad("BRANCH", branch_w + 2));
        head.push_str(&pad("MERGED", via_w));
        head.push_str(&rpad("SIZE", size_w));
        head.push_str("  PATH");
        lines.push(Line::from(Span::styled(head, t.fg(t.faint))));
        let height = (area.height as usize).saturating_sub(5);
        let offset = scroll_offset(app.gc_sel, height);
        for (i, item) in r.would_remove.iter().enumerate().skip(offset).take(height) {
            let sel = i == app.gc_sel;
            let checked = app.gc_checked.get(i).copied().unwrap_or(false);
            let size = match app.size_of(&item.path) {
                Some(b) => human_bytes(b),
                None => spinner(app).to_string(),
            };
            let mut spans = vec![
                Span::styled(if sel { "➤ " } else { "  " }, t.fg(t.accent)),
                Span::styled(
                    if checked { "◉ " } else { "○ " },
                    if checked {
                        t.fg(t.accent)
                    } else {
                        t.fg(t.faint)
                    },
                ),
            ];
            if app.all {
                spans.push(Span::styled(pad(&item.repo, repo_w), t.fg(t.dim)));
            }
            spans.extend([
                Span::styled(
                    pad(item.branch.as_deref().unwrap_or("(detached)"), branch_w + 2),
                    if sel { t.accent_bold() } else { Style::new() },
                ),
                Span::styled(
                    pad(item.merged_via.as_deref().unwrap_or("yes"), via_w),
                    t.fg(t.dim),
                ),
                Span::styled(rpad(&size, size_w), t.fg(t.dim)),
                Span::styled(
                    format!("  {}", truncate(&tilde(&item.path), path_w)),
                    t.fg(t.faint),
                ),
            ]);
            let mut line = Line::from(spans);
            if sel {
                line = highlight(line, area.width, t);
            }
            lines.push(line);
        }
        let picked: Vec<_> = r
            .would_remove
            .iter()
            .zip(&app.gc_checked)
            .filter(|(_, c)| **c)
            .collect();
        let known: u64 = picked
            .iter()
            .filter_map(|(i, _)| app.size_of(&i.path))
            .sum();
        lines.push(Line::default());
        lines.push(Line::from(vec![
            Span::styled(format!("{} selected  ", picked.len()), t.fg(t.dim)),
            Span::styled(human_bytes(known), t.accent_bold()),
            Span::styled("  apparent size", t.fg(t.faint)),
        ]));
    }
    lines.push(Line::default());
    let mut notes = Vec::new();
    if !r.skipped.is_empty() {
        notes.push(format!(
            "{} kept with uncommitted or unpushed work",
            r.skipped.len()
        ));
    }
    if !notes.is_empty() {
        lines.push(Line::from(Span::styled(notes.join(" · "), t.fg(t.faint))));
    }
    f.render_widget(Paragraph::new(lines), area);
}

fn slim(f: &mut Frame, area: Rect, app: &App, t: &Theme) {
    let mut lines = vec![Line::from(vec![
        Span::styled("Idle for  ", t.fg(t.dim)),
        Span::styled(
            if app.slim_idle == IDLE_STEPS[0] {
                "  "
            } else {
                "‹ "
            },
            t.fg(t.accent),
        ),
        Span::styled(idle_label(app.slim_idle), t.accent_bold()),
        Span::styled(
            if app.slim_idle == IDLE_STEPS[IDLE_STEPS.len() - 1] {
                ""
            } else {
                " ›"
            },
            t.fg(t.accent),
        ),
    ])];
    lines.push(Line::default());
    match &app.slim {
        Load::Loading => lines.push(loading_line(app, t, "Measuring build caches…")),
        Load::Failed(e) => lines.push(failed_line(t, e)),
        Load::Ready(r) if r.worktrees.is_empty() => lines.push(Line::from(vec![
            Span::styled("✓ ", t.fg(t.accent)),
            Span::raw("No reclaimable caches in idle worktrees."),
        ])),
        Load::Ready(r) => {
            let branch_w = r
                .worktrees
                .iter()
                .map(|w| w.branch.as_deref().unwrap_or("(detached)").chars().count())
                .max()
                .unwrap_or(10)
                .clamp(10, 40);
            let height = (area.height as usize).saturating_sub(10);
            let offset = scroll_offset(app.slim_sel, height.max(1));
            for (i, w) in r
                .worktrees
                .iter()
                .enumerate()
                .skip(offset)
                .take(height.max(1))
            {
                let sel = i == app.slim_sel;
                let bytes: u64 = w.items.iter().map(|i| i.apparent_bytes).sum();
                let mut line = Line::from(vec![
                    Span::styled(if sel { "➤ " } else { "  " }, t.fg(t.accent)),
                    Span::styled(
                        pad(w.branch.as_deref().unwrap_or("(detached)"), branch_w + 2),
                        if sel { t.accent_bold() } else { Style::new() },
                    ),
                    Span::styled(
                        pad(
                            &format!("{} cache{}", w.items.len(), plural(w.items.len())),
                            12,
                        ),
                        t.fg(t.dim),
                    ),
                    Span::styled(rpad(&human_bytes(bytes), 10), t.fg(t.accent)),
                ]);
                if sel {
                    line = highlight(line, area.width, t);
                }
                lines.push(line);
            }
            if let Some(w) = r.worktrees.get(app.slim_sel) {
                lines.push(Line::default());
                for item in w.items.iter().take(4) {
                    let rel = item
                        .path
                        .strip_prefix(&w.path)
                        .unwrap_or(&item.path)
                        .display()
                        .to_string();
                    lines.push(Line::from(vec![
                        Span::styled(rpad(&human_bytes(item.apparent_bytes), 12), t.fg(t.faint)),
                        Span::styled(format!("  {rel}"), t.fg(t.faint)),
                    ]));
                }
                if w.items.len() > 4 {
                    lines.push(Line::from(Span::styled(
                        format!("{:>12}  and {} more", "", w.items.len() - 4),
                        t.fg(t.faint),
                    )));
                }
            }
            lines.push(Line::default());
            lines.push(Line::from(vec![
                Span::styled("Total  ", t.fg(t.dim)),
                Span::styled(human_bytes(r.total_apparent_bytes), t.accent_bold()),
                Span::styled("  apparent size", t.fg(t.faint)),
            ]));
        }
    }
    f.render_widget(Paragraph::new(lines), area);
}

fn setup(f: &mut Frame, area: Rect, app: &App, t: &Theme) {
    let title_w = app.setup.iter().map(|s| s.title.len()).max().unwrap_or(10) + 4;
    let mut lines = Vec::new();
    for (i, s) in app.setup.iter().enumerate() {
        let sel = i == app.setup_sel;
        let (mark, mark_style) = match s.tone {
            None => ("→ ", t.fg(t.faint)),
            Some(Tone::Ok) => ("● ", t.fg(t.accent)),
            Some(Tone::Warn) => ("● ", t.fg(t.warn)),
            Some(Tone::Error) => ("● ", t.fg(t.danger)),
        };
        let mut line = Line::from(vec![
            Span::styled(if sel { "➤ " } else { "  " }, t.fg(t.accent)),
            Span::styled(
                pad(s.title, title_w),
                if sel { t.accent_bold() } else { Style::new() },
            ),
            Span::styled(mark, mark_style),
            Span::styled(s.status.clone(), t.fg(t.dim)),
        ]);
        if sel {
            line = highlight(line, area.width, t);
        }
        lines.push(line);
    }
    f.render_widget(Paragraph::new(lines), area);
}

// ---------------------------------------------------------------------------------------------
// Overlays
// ---------------------------------------------------------------------------------------------

fn render_toast(f: &mut Frame, area: Rect, app: &App, t: &Theme) {
    let Some((msg, tone, _)) = &app.toast else {
        return;
    };
    let (icon, style) = match tone {
        Tone::Ok => ("✓", t.fg(t.accent)),
        Tone::Warn => ("!", t.fg(t.warn)),
        Tone::Error => ("✗", t.fg(t.danger)),
    };
    let area = Rect {
        x: area.x + PAD,
        width: area.width.saturating_sub(PAD),
        ..area
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!("{icon} "), style),
            Span::styled(msg.clone(), style),
        ])),
        area,
    );
}

fn boxed<'a>(title: Line<'a>, border: Style) -> Block<'a> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border)
        .title(title)
        .padding(ratatui::widgets::Padding::new(2, 2, 1, 0))
}

fn wrapped_height(lines: &[Line], width: u16) -> u16 {
    let w = width.max(1) as usize;
    lines
        .iter()
        .map(|l| (l.width().max(1)).div_ceil(w) as u16)
        .sum()
}

fn modal(f: &mut Frame, area: Rect, m: &Modal, t: &Theme) {
    let width = area.width.saturating_sub(6).min(76);
    let inner_w = width.saturating_sub(6);
    let (title, border, mut body, keys): (Line, Style, Vec<Line>, Vec<(&str, &str)>) = match m {
        Modal::Confirm {
            title,
            lines,
            danger,
            ..
        } => {
            let c = if *danger { t.danger } else { t.accent };
            (
                Line::from(Span::styled(
                    format!(" {title} "),
                    t.fg(c).add_modifier(Modifier::BOLD),
                )),
                t.fg(c),
                lines
                    .iter()
                    .map(|l| Line::from(Span::raw(tildify(l))))
                    .collect(),
                vec![("Enter", "Confirm"), ("Esc", "Cancel")],
            )
        }
        Modal::TypeToConfirm {
            title,
            lines,
            expect,
            input,
            ..
        } => {
            let mut body: Vec<Line> = lines
                .iter()
                .map(|l| Line::from(Span::raw(tildify(l))))
                .collect();
            body.push(Line::default());
            body.push(Line::from(vec![
                Span::styled("Type ", t.fg(t.dim)),
                Span::styled(expect.clone(), Style::new().add_modifier(Modifier::BOLD)),
                Span::styled(" to confirm", t.fg(t.dim)),
            ]));
            let matches = input == expect;
            body.push(Line::from(vec![
                Span::styled("› ", t.fg(t.danger)),
                Span::styled(
                    input.clone(),
                    if matches {
                        t.fg(t.danger).add_modifier(Modifier::BOLD)
                    } else {
                        Style::new()
                    },
                ),
                Span::styled("▏", t.fg(t.danger)),
            ]));
            (
                Line::from(Span::styled(
                    format!(" {title} "),
                    t.fg(t.danger).add_modifier(Modifier::BOLD),
                )),
                t.fg(t.danger),
                body,
                if matches {
                    vec![("Enter", "Delete"), ("Esc", "Cancel")]
                } else {
                    vec![("Esc", "Cancel")]
                },
            )
        }
        Modal::Result {
            title,
            text,
            tone,
            open,
            scroll,
        } => {
            let (icon, c) = match tone {
                Tone::Ok => ("✓", t.accent),
                Tone::Warn => ("!", t.warn),
                Tone::Error => ("✗", t.danger),
            };
            let body: Vec<Line> = text
                .lines()
                .skip(*scroll as usize)
                .map(|l| {
                    let style = if l.starts_with("warning:") {
                        t.fg(t.warn)
                    } else if l.starts_with("fix:") {
                        t.fg(t.accent)
                    } else {
                        Style::new()
                    };
                    Line::from(Span::styled(tildify(l), style))
                })
                .collect();
            let mut keys = Vec::new();
            if open.is_some() {
                keys.push(("Enter", "Open"));
            }
            if text.lines().count() > 12 {
                keys.push(("↑↓", "Scroll"));
            }
            keys.push(("Esc", "Close"));
            (
                Line::from(Span::styled(
                    format!(" {icon} {title} "),
                    t.fg(c).add_modifier(Modifier::BOLD),
                )),
                t.fg(c),
                body,
                keys,
            )
        }
    };
    body.push(Line::default());
    body.push(footer(&keys, t));
    let max_h = area.height.saturating_sub(4);
    let height = (wrapped_height(&body, inner_w) + 3).min(max_h);
    let rect = center(area, width, height);
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(Text::from(body))
            .wrap(Wrap { trim: false })
            .block(boxed(title, border)),
        rect,
    );
}

fn busy(f: &mut Frame, area: Rect, app: &App, t: &Theme) {
    let Some(b) = &app.busy else {
        return;
    };
    let width = area.width.saturating_sub(6).min(76);
    let secs = b.started.elapsed().as_secs();
    let mut body = vec![Line::from(vec![
        Span::styled(format!("{} ", spinner(app)), t.fg(t.accent)),
        Span::styled(b.label.clone(), Style::new().add_modifier(Modifier::BOLD)),
        Span::styled(format!("   {secs}s"), t.fg(t.faint)),
    ])];
    body.push(Line::default());
    let inner_w = width.saturating_sub(6) as usize;
    for l in b.lines.iter().rev().take(6).rev() {
        let style = if l.starts_with("warning:") {
            t.fg(t.warn)
        } else {
            t.fg(t.dim)
        };
        body.push(Line::from(Span::styled(
            truncate(&tildify(l), inner_w),
            style,
        )));
    }
    let height = body.len() as u16 + 3;
    let rect = center(area, width, height.max(6));
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(body).block(boxed(Line::default(), t.fg(t.accent))),
        rect,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logo_rows_share_a_width() {
        for row in LOGO {
            assert_eq!(row.chars().count() as u16, LOGO_W, "{row}");
        }
    }

    #[test]
    fn tildify_only_touches_home_paths() {
        let home = paths::home().display().to_string();
        assert_eq!(
            tildify(&format!("rm {home}/wt and {home}x/y")),
            format!("rm ~/wt and {home}x/y")
        );
    }

    #[test]
    fn truncate_and_pad() {
        assert_eq!(truncate("feature/long", 5), "feat…");
        assert_eq!(pad("ab", 4), "ab  ");
        assert_eq!(rpad("ab", 4), "  ab");
        assert_eq!(scroll_offset(3, 5), 0);
        assert_eq!(scroll_offset(7, 5), 3);
    }
}
