use std::io::{stdout, Stdout};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState},
    Frame, Terminal,
};

use crate::{config, fleet, git, isolation, sync};

// ── Palette ──────────────────────────────────────────────────────────────

const ACCENT: Color = Color::Rgb(212, 255, 0);
const RUNNING: Color = Color::Rgb(134, 239, 172);
const MODIFIED: Color = Color::Rgb(251, 191, 36);
const ERROR_COLOR: Color = Color::Rgb(239, 68, 68);
const MUTED: Color = Color::Rgb(80, 80, 80);
const DIM: Color = Color::Rgb(38, 38, 38);
const TEXT: Color = Color::Rgb(220, 220, 220);
const SEL_BG: Color = Color::Rgb(20, 20, 20);
const BG: Color = Color::Rgb(8, 8, 8);

// ── Panels ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum Panel {
    Worktrees,
    Fleet,
    Files,
    Isolation,
}

impl Panel {
    fn next(self) -> Self {
        match self {
            Panel::Worktrees => Panel::Fleet,
            Panel::Fleet => Panel::Files,
            Panel::Files => Panel::Isolation,
            Panel::Isolation => Panel::Worktrees,
        }
    }
    fn prev(self) -> Self {
        match self {
            Panel::Worktrees => Panel::Isolation,
            Panel::Fleet => Panel::Worktrees,
            Panel::Files => Panel::Fleet,
            Panel::Isolation => Panel::Files,
        }
    }
    fn title(self) -> &'static str {
        match self {
            Panel::Worktrees => " Worktrees ",
            Panel::Fleet => " Fleet ",
            Panel::Files => " Files ",
            Panel::Isolation => " Ports ",
        }
    }
}

// ── Data rows ────────────────────────────────────────────────────────────

struct WorktreeRow {
    branch: String,
    path: PathBuf,
    is_bare: bool,
    is_dirty: bool,
    last_commit: Option<String>,
    port_info: Option<String>,
}

#[allow(dead_code)]
struct FleetRow {
    branch: String,
    task: String,
    path: PathBuf,
    exists: bool,
    is_dirty: bool,
    agent_pid: Option<u32>,
}

struct FileRow {
    status: String,
    path: String,
}

#[allow(dead_code)]
struct IsolationRow {
    branch: String,
    port: u16,
    port_end: u16,
    db_name: String,
    compose_project: String,
}

// ── Input / Confirm modes ────────────────────────────────────────────────

enum InputAction {
    NewWorktree,
}

enum ConfirmAction {
    DeleteWorktree(String, PathBuf),
}

enum InputMode {
    Normal,
    Input {
        prompt: String,
        buf: String,
        action: InputAction,
    },
    Confirm {
        msg: String,
        action: ConfirmAction,
    },
    Help,
}

// ── Agent detection ──────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn detect_agent(wt_path: &std::path::Path) -> Option<u32> {
    let procs = std::fs::read_dir("/proc").ok()?;
    for entry in procs.flatten() {
        let fname = entry.file_name();
        let pid_str = fname.to_string_lossy();
        if !pid_str.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let cmdline_path = entry.path().join("cmdline");
        let Ok(bytes) = std::fs::read(&cmdline_path) else { continue };
        let cmdline = String::from_utf8_lossy(&bytes).to_lowercase();
        if !cmdline.contains("claude") && !cmdline.contains("aider") && !cmdline.contains("codex") {
            continue;
        }
        let cwd_link = entry.path().join("cwd");
        let Ok(cwd) = std::fs::read_link(&cwd_link) else { continue };
        if cwd == wt_path {
            return pid_str.parse().ok();
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn detect_agent(wt_path: &std::path::Path) -> Option<u32> {
    for agent in &["claude", "aider", "codex"] {
        let pgrep = std::process::Command::new("pgrep")
            .args(["-f", agent])
            .output()
            .ok()?;
        if !pgrep.status.success() { continue; }
        for pid_str in String::from_utf8_lossy(&pgrep.stdout).split_whitespace() {
            let lsof = std::process::Command::new("lsof")
                .args(["-a", "-p", pid_str, "-d", "cwd", "-Fn"])
                .output()
                .ok()?;
            for line in String::from_utf8_lossy(&lsof.stdout).lines() {
                if let Some(path) = line.strip_prefix('n') {
                    if std::path::Path::new(path) == wt_path {
                        return pid_str.parse().ok();
                    }
                }
            }
        }
    }
    None
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn detect_agent(_wt_path: &std::path::Path) -> Option<u32> {
    None
}

// ── App state ────────────────────────────────────────────────────────────

struct DashboardApp {
    panel: Panel,
    worktrees: Vec<WorktreeRow>,
    fleet_rows: Vec<FleetRow>,
    files: Vec<FileRow>,
    isolation_rows: Vec<IsolationRow>,
    fleet_agent: String,
    wt_selected: usize,
    fleet_selected: usize,
    mode: InputMode,
    last_refresh: Instant,
    wt_table: TableState,
    fleet_table: TableState,
    status_msg: Option<(String, Instant)>,
    repo_root: Option<PathBuf>,
}

impl DashboardApp {
    fn new() -> Self {
        let mut app = Self {
            panel: Panel::Worktrees,
            worktrees: Vec::new(),
            fleet_rows: Vec::new(),
            files: Vec::new(),
            isolation_rows: Vec::new(),
            fleet_agent: String::new(),
            wt_selected: 0,
            fleet_selected: 0,
            mode: InputMode::Normal,
            last_refresh: Instant::now(),
            wt_table: TableState::default(),
            fleet_table: TableState::default(),
            status_msg: None,
            repo_root: git::repo_root().ok(),
        };
        app.refresh();
        app
    }

    fn refresh(&mut self) {
        self.refresh_worktrees();
        self.refresh_fleet();
        self.refresh_isolation();
        self.refresh_files();
        self.last_refresh = Instant::now();
    }

    fn refresh_worktrees(&mut self) {
        let registry = isolation::load_registry();
        let worktrees = git::worktree_list().unwrap_or_default();
        self.worktrees = worktrees
            .into_iter()
            .map(|wt| {
                let is_dirty = if wt.is_bare {
                    false
                } else {
                    git::is_dirty(&wt.path).unwrap_or(false)
                };
                let last_commit = if wt.is_bare {
                    None
                } else {
                    git::last_commit_relative(&wt.path)
                };
                let port_info = registry
                    .allocations
                    .values()
                    .find(|a| a.branch == wt.branch)
                    .map(|a| {
                        if a.port_count > 1 {
                            format!("{}-{}", a.port, a.port + a.port_count - 1)
                        } else {
                            format!("{}", a.port)
                        }
                    });
                WorktreeRow {
                    branch: wt.branch,
                    path: wt.path,
                    is_bare: wt.is_bare,
                    is_dirty,
                    last_commit,
                    port_info,
                }
            })
            .collect();

        if !self.worktrees.is_empty() {
            self.wt_selected = self.wt_selected.min(self.worktrees.len() - 1);
            self.wt_table.select(Some(self.wt_selected));
        } else {
            self.wt_table.select(None);
        }
    }

    fn refresh_fleet(&mut self) {
        let root = match &self.repo_root {
            Some(r) => r,
            None => return,
        };
        if let Some(state) = fleet::try_load_state(root) {
            self.fleet_agent = state.agent.clone();
            self.fleet_rows = state
                .tasks
                .iter()
                .map(|ft| {
                    let path = PathBuf::from(&ft.path);
                    let exists = path.exists();
                    let is_dirty = exists && git::is_dirty(&path).unwrap_or(false);
                    let agent_pid = if exists { detect_agent(&path) } else { None };
                    FleetRow {
                        branch: ft.branch.clone(),
                        task: ft.task.clone(),
                        path,
                        exists,
                        is_dirty,
                        agent_pid,
                    }
                })
                .collect();
            if !self.fleet_rows.is_empty() {
                self.fleet_selected = self.fleet_selected.min(self.fleet_rows.len() - 1);
                self.fleet_table.select(Some(self.fleet_selected));
            }
        } else {
            self.fleet_rows.clear();
            self.fleet_agent.clear();
            self.fleet_table.select(None);
        }
    }

    fn refresh_files(&mut self) {
        self.files.clear();
        if self.worktrees.is_empty() {
            return;
        }
        let wt = &self.worktrees[self.wt_selected];
        if wt.is_bare {
            return;
        }
        if let Ok(files) = git::modified_files_with_status(&wt.path) {
            self.files = files
                .into_iter()
                .map(|(status, path)| FileRow { status, path })
                .collect();
        }
    }

    fn refresh_isolation(&mut self) {
        let registry = isolation::load_registry();
        self.isolation_rows = registry
            .allocations
            .values()
            .map(|a| IsolationRow {
                branch: a.branch.clone(),
                port: a.port,
                port_end: a.port + a.port_count - 1,
                db_name: a.db_name.clone(),
                compose_project: a.compose_project.clone(),
            })
            .collect();
        self.isolation_rows.sort_by_key(|r| r.port);
    }

    fn selected_worktree(&self) -> Option<&WorktreeRow> {
        self.worktrees.get(self.wt_selected)
    }

    fn set_status(&mut self, msg: impl Into<String>) {
        self.status_msg = Some((msg.into(), Instant::now()));
    }

    fn nav_up(&mut self) {
        match self.panel {
            Panel::Worktrees => {
                if !self.worktrees.is_empty() {
                    self.wt_selected = self.wt_selected.saturating_sub(1);
                    self.wt_table.select(Some(self.wt_selected));
                    self.refresh_files();
                }
            }
            Panel::Fleet => {
                if !self.fleet_rows.is_empty() {
                    self.fleet_selected = self.fleet_selected.saturating_sub(1);
                    self.fleet_table.select(Some(self.fleet_selected));
                }
            }
            _ => {}
        }
    }

    fn nav_down(&mut self) {
        match self.panel {
            Panel::Worktrees => {
                if !self.worktrees.is_empty() {
                    self.wt_selected = (self.wt_selected + 1).min(self.worktrees.len() - 1);
                    self.wt_table.select(Some(self.wt_selected));
                    self.refresh_files();
                }
            }
            Panel::Fleet => {
                if !self.fleet_rows.is_empty() {
                    self.fleet_selected = (self.fleet_selected + 1).min(self.fleet_rows.len() - 1);
                    self.fleet_table.select(Some(self.fleet_selected));
                }
            }
            _ => {}
        }
    }
}

// ── Entry point ──────────────────────────────────────────────────────────

pub fn run_dashboard() -> Result<()> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;

    let result = dashboard_loop(&mut terminal);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn dashboard_loop(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    let mut app = DashboardApp::new();
    const TICK: Duration = Duration::from_millis(100);
    const REFRESH: Duration = Duration::from_secs(2);

    loop {
        terminal.draw(|f| draw_dashboard(f, &mut app))?;

        if event::poll(TICK)? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                match &app.mode {
                    InputMode::Normal => match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Tab => {
                            app.panel = if key.modifiers.contains(KeyModifiers::SHIFT) {
                                app.panel.prev()
                            } else {
                                app.panel.next()
                            };
                        }
                        KeyCode::Up | KeyCode::Char('k') => app.nav_up(),
                        KeyCode::Down | KeyCode::Char('j') => app.nav_down(),
                        KeyCode::Char('r') => {
                            app.refresh();
                            app.set_status("refreshed");
                        }
                        KeyCode::Char('n') => {
                            app.mode = InputMode::Input {
                                prompt: "New worktree branch: ".to_string(),
                                buf: String::new(),
                                action: InputAction::NewWorktree,
                            };
                        }
                        KeyCode::Char('d') => {
                            if let Some(wt) = app.selected_worktree() {
                                if !wt.is_bare {
                                    let branch = wt.branch.clone();
                                    let path = wt.path.clone();
                                    app.mode = InputMode::Confirm {
                                        msg: format!("Delete worktree '{}'? (y/n)", branch),
                                        action: ConfirmAction::DeleteWorktree(branch, path),
                                    };
                                }
                            }
                        }
                        KeyCode::Char('s') => {
                            if let Some(wt) = app.selected_worktree() {
                                if !wt.is_bare {
                                    let path = wt.path.clone();
                                    if let Some(root) = &app.repo_root {
                                        let config = config::load_config(root).unwrap_or_default();
                                        match sync::sync_worktree(root, &path, &config.sync) {
                                            Ok(_) => app.set_status("synced"),
                                            Err(e) => app.set_status(format!("sync error: {e}")),
                                        }
                                        app.refresh();
                                    }
                                }
                            }
                        }
                        KeyCode::Char('?') => {
                            app.mode = InputMode::Help;
                        }
                        _ => {}
                    },
                    InputMode::Input { .. } => match key.code {
                        KeyCode::Esc => app.mode = InputMode::Normal,
                        KeyCode::Enter => {
                            if let InputMode::Input { buf, action, .. } =
                                std::mem::replace(&mut app.mode, InputMode::Normal)
                            {
                                handle_input_action(&mut app, action, &buf);
                            }
                        }
                        KeyCode::Backspace => {
                            if let InputMode::Input { buf, .. } = &mut app.mode {
                                buf.pop();
                            }
                        }
                        KeyCode::Char(c) => {
                            if let InputMode::Input { buf, .. } = &mut app.mode {
                                buf.push(c);
                            }
                        }
                        _ => {}
                    },
                    InputMode::Confirm { .. } => match key.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            if let InputMode::Confirm { action, .. } =
                                std::mem::replace(&mut app.mode, InputMode::Normal)
                            {
                                handle_confirm_action(&mut app, action);
                            }
                        }
                        _ => app.mode = InputMode::Normal,
                    },
                    InputMode::Help => {
                        app.mode = InputMode::Normal;
                    }
                }
            }
        }

        if app.last_refresh.elapsed() >= REFRESH {
            app.refresh();
        }

        // Clear status after 3s
        if let Some((_, t)) = &app.status_msg {
            if t.elapsed() > Duration::from_secs(3) {
                app.status_msg = None;
            }
        }
    }
    Ok(())
}

// ── Action handlers ──────────────────────────────────────────────────────

fn handle_input_action(app: &mut DashboardApp, action: InputAction, input: &str) {
    if input.trim().is_empty() {
        return;
    }
    match action {
        InputAction::NewWorktree => {
            let branch = input.trim();
            if let Some(root) = &app.repo_root {
                let wt_path = git::worktree_path(root, branch);
                if wt_path.exists() {
                    app.set_status(format!("'{}' already exists", branch));
                    return;
                }
                match git::worktree_add(&wt_path, branch, None) {
                    Ok(_) => {
                        let config = config::load_config(root).unwrap_or_default();
                        let _ = sync::sync_worktree(root, &wt_path, &config.sync);
                        app.set_status(format!("created '{}'", branch));
                        app.refresh();
                    }
                    Err(e) => app.set_status(format!("error: {e}")),
                }
            }
        }
    }
}

fn handle_confirm_action(app: &mut DashboardApp, action: ConfirmAction) {
    match action {
        ConfirmAction::DeleteWorktree(branch, path) => {
            let force = git::is_dirty(&path).unwrap_or(false);
            match git::worktree_remove(&path, force) {
                Ok(_) => {
                    app.set_status(format!("removed '{}'", branch));
                    app.refresh();
                }
                Err(e) => app.set_status(format!("error: {e}")),
            }
        }
    }
}

// ── Main draw ────────────────────────────────────────────────────────────

fn draw_dashboard(f: &mut Frame, app: &mut DashboardApp) {
    let area = f.area();
    f.render_widget(Block::default().style(Style::default().bg(BG)), area);

    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // header
            Constraint::Min(6),   // panels
            Constraint::Length(1), // footer
        ])
        .split(area);

    draw_header(f, app, main_chunks[0]);

    // 2x2 grid
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(main_chunks[1]);

    let top_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(rows[0]);

    let bot_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(rows[1]);

    draw_worktrees_panel(f, app, top_cols[0]);
    draw_fleet_panel(f, app, top_cols[1]);
    draw_files_panel(f, app, bot_cols[0]);
    draw_isolation_panel(f, app, bot_cols[1]);

    draw_footer(f, app, main_chunks[2]);

    // Overlays
    match &app.mode {
        InputMode::Help => draw_help_overlay(f, area),
        InputMode::Input { prompt, buf, .. } => draw_input_overlay(f, area, prompt, buf),
        InputMode::Confirm { msg, .. } => draw_confirm_overlay(f, area, msg),
        InputMode::Normal => {}
    }
}

// ── Header ───────────────────────────────────────────────────────────────

fn draw_header(f: &mut Frame, app: &DashboardApp, area: Rect) {
    let wt_count = app.worktrees.iter().filter(|w| !w.is_bare).count();
    let dirty = app.worktrees.iter().filter(|w| w.is_dirty).count();

    let mut spans = vec![
        Span::styled(" workz", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::styled("  ", Style::default().fg(DIM)),
        Span::styled(
            format!("{} worktree{}", wt_count, if wt_count == 1 { "" } else { "s" }),
            Style::default().fg(TEXT),
        ),
    ];

    if dirty > 0 {
        spans.push(Span::styled("  ", Style::default().fg(DIM)));
        spans.push(Span::styled(
            format!("{} dirty", dirty),
            Style::default().fg(MODIFIED),
        ));
    }

    if !app.fleet_rows.is_empty() {
        let running = app.fleet_rows.iter().filter(|r| r.agent_pid.is_some()).count();
        spans.push(Span::styled("  ", Style::default().fg(DIM)));
        spans.push(Span::styled(
            format!("{} fleet", app.fleet_rows.len()),
            Style::default().fg(TEXT),
        ));
        if running > 0 {
            spans.push(Span::styled(
                format!(" ({} running)", running),
                Style::default().fg(RUNNING),
            ));
        }
    }

    if let Some((msg, _)) = &app.status_msg {
        spans.push(Span::styled("  ", Style::default().fg(DIM)));
        spans.push(Span::styled(
            format!("[{}]", msg),
            Style::default().fg(ACCENT),
        ));
    }

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ── Worktrees panel ──────────────────────────────────────────────────────

fn draw_worktrees_panel(f: &mut Frame, app: &mut DashboardApp, area: Rect) {
    let active = app.panel == Panel::Worktrees;
    let block = panel_block(Panel::Worktrees, active);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.worktrees.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled("  no worktrees", Style::default().fg(MUTED)))),
            inner,
        );
        return;
    }

    let header = Row::new(vec![
        Cell::from("  branch").style(Style::default().fg(MUTED)),
        Cell::from("status").style(Style::default().fg(MUTED)),
        Cell::from("port").style(Style::default().fg(MUTED)),
        Cell::from("commit").style(Style::default().fg(MUTED)),
    ])
    .height(1);

    let rows: Vec<Row> = app
        .worktrees
        .iter()
        .enumerate()
        .map(|(i, wt)| {
            let sel = i == app.wt_selected && active;
            let branch_style = if sel {
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Rgb(175, 175, 175))
            };

            let (status_str, status_color) = if wt.is_bare {
                ("bare", MUTED)
            } else if wt.is_dirty {
                ("modified", MODIFIED)
            } else {
                ("clean", RUNNING)
            };

            let port_str = wt.port_info.as_deref().unwrap_or("-");

            let commit_str = wt.last_commit.as_deref().unwrap_or("-");

            Row::new(vec![
                Cell::from(format!("  {}", truncate(&wt.branch, 28))).style(branch_style),
                Cell::from(status_str).style(Style::default().fg(status_color)),
                Cell::from(truncate(port_str, 12)).style(Style::default().fg(Color::Rgb(100, 100, 100))),
                Cell::from(truncate(commit_str, 14)).style(Style::default().fg(Color::Rgb(60, 60, 60))),
            ])
            .style(if i == app.wt_selected {
                Style::default().bg(SEL_BG)
            } else {
                Style::default()
            })
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Min(24),
            Constraint::Length(10),
            Constraint::Length(14),
            Constraint::Length(14),
        ],
    )
    .header(header)
    .highlight_style(Style::default().bg(SEL_BG));

    f.render_stateful_widget(table, inner, &mut app.wt_table);
}

// ── Fleet panel ──────────────────────────────────────────────────────────

fn draw_fleet_panel(f: &mut Frame, app: &mut DashboardApp, area: Rect) {
    let active = app.panel == Panel::Fleet;
    let block = panel_block(Panel::Fleet, active);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.fleet_rows.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled("  no fleet active", Style::default().fg(MUTED)))),
            inner,
        );
        return;
    }

    let header = Row::new(vec![
        Cell::from("  branch").style(Style::default().fg(MUTED)),
        Cell::from("task").style(Style::default().fg(MUTED)),
        Cell::from("status").style(Style::default().fg(MUTED)),
    ])
    .height(1);

    let rows: Vec<Row> = app
        .fleet_rows
        .iter()
        .enumerate()
        .map(|(i, fr)| {
            let sel = i == app.fleet_selected && active;
            let branch_style = if sel {
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Rgb(175, 175, 175))
            };

            let (status_str, status_color) = if !fr.exists {
                ("missing", ERROR_COLOR)
            } else if fr.agent_pid.is_some() {
                ("running", RUNNING)
            } else if fr.is_dirty {
                ("modified", MODIFIED)
            } else {
                ("clean", MUTED)
            };

            Row::new(vec![
                Cell::from(format!("  {}", truncate(&fr.branch, 22))).style(branch_style),
                Cell::from(truncate(&fr.task, 20)).style(Style::default().fg(Color::Rgb(110, 110, 110))),
                Cell::from(status_str).style(Style::default().fg(status_color)),
            ])
            .style(if i == app.fleet_selected {
                Style::default().bg(SEL_BG)
            } else {
                Style::default()
            })
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Min(18),
            Constraint::Min(16),
            Constraint::Length(10),
        ],
    )
    .header(header)
    .highlight_style(Style::default().bg(SEL_BG));

    f.render_stateful_widget(table, inner, &mut app.fleet_table);
}

// ── Files panel ──────────────────────────────────────────────────────────

fn draw_files_panel(f: &mut Frame, app: &mut DashboardApp, area: Rect) {
    let active = app.panel == Panel::Files;
    let title = if let Some(wt) = app.selected_worktree() {
        format!(" Files ({}) ", wt.branch)
    } else {
        " Files ".to_string()
    };

    let border_color = if active { ACCENT } else { DIM };
    let block = Block::default()
        .title(Span::styled(
            title,
            Style::default().fg(if active { ACCENT } else { MUTED }),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.files.is_empty() {
        let msg = if app.worktrees.is_empty() {
            "  no worktrees"
        } else {
            "  clean"
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(msg, Style::default().fg(MUTED)))),
            inner,
        );
        return;
    }

    let lines: Vec<Line> = app
        .files
        .iter()
        .map(|fr| {
            let status_color = match fr.status.as_str() {
                "M" => MODIFIED,
                "A" => RUNNING,
                "D" => ERROR_COLOR,
                "??" => Color::Rgb(100, 100, 100),
                _ => TEXT,
            };
            Line::from(vec![
                Span::styled(format!("  {} ", fr.status), Style::default().fg(status_color)),
                Span::styled(&fr.path, Style::default().fg(Color::Rgb(160, 160, 160))),
            ])
        })
        .collect();

    f.render_widget(Paragraph::new(lines), inner);
}

// ── Isolation panel ──────────────────────────────────────────────────────

fn draw_isolation_panel(f: &mut Frame, app: &mut DashboardApp, area: Rect) {
    let active = app.panel == Panel::Isolation;
    let block = panel_block(Panel::Isolation, active);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.isolation_rows.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  no isolated worktrees",
                Style::default().fg(MUTED),
            ))),
            inner,
        );
        return;
    }

    let lines: Vec<Line> = app
        .isolation_rows
        .iter()
        .map(|ir| {
            let port_str = if ir.port == ir.port_end {
                format!("{}", ir.port)
            } else {
                format!("{}-{}", ir.port, ir.port_end)
            };
            Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    truncate(&ir.branch, 20),
                    Style::default().fg(Color::Rgb(175, 175, 175)),
                ),
                Span::styled("  PORT:", Style::default().fg(MUTED)),
                Span::styled(port_str, Style::default().fg(ACCENT)),
                Span::styled("  DB:", Style::default().fg(MUTED)),
                Span::styled(
                    truncate(&ir.db_name, 16),
                    Style::default().fg(Color::Rgb(140, 140, 140)),
                ),
            ])
        })
        .collect();

    f.render_widget(Paragraph::new(lines), inner);
}

// ── Footer ───────────────────────────────────────────────────────────────

fn draw_footer(f: &mut Frame, _app: &DashboardApp, area: Rect) {
    let line = Line::from(vec![
        Span::styled(" Tab", Style::default().fg(ACCENT)),
        Span::styled(" panel  ", Style::default().fg(MUTED)),
        Span::styled("j/k", Style::default().fg(ACCENT)),
        Span::styled(" nav  ", Style::default().fg(MUTED)),
        Span::styled("n", Style::default().fg(ACCENT)),
        Span::styled(" new  ", Style::default().fg(MUTED)),
        Span::styled("d", Style::default().fg(ACCENT)),
        Span::styled(" delete  ", Style::default().fg(MUTED)),
        Span::styled("s", Style::default().fg(ACCENT)),
        Span::styled(" sync  ", Style::default().fg(MUTED)),
        Span::styled("r", Style::default().fg(ACCENT)),
        Span::styled(" refresh  ", Style::default().fg(MUTED)),
        Span::styled("?", Style::default().fg(ACCENT)),
        Span::styled(" help  ", Style::default().fg(MUTED)),
        Span::styled("q", Style::default().fg(ACCENT)),
        Span::styled(" quit", Style::default().fg(MUTED)),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

// ── Overlays ─────────────────────────────────────────────────────────────

fn draw_help_overlay(f: &mut Frame, area: Rect) {
    let popup = centered_rect(50, 60, area);
    f.render_widget(Clear, popup);

    let help_text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Tab / Shift+Tab", Style::default().fg(ACCENT)),
            Span::styled("  cycle panels", Style::default().fg(TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  j / k / arrows ", Style::default().fg(ACCENT)),
            Span::styled("  navigate list", Style::default().fg(TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  n              ", Style::default().fg(ACCENT)),
            Span::styled("  new worktree", Style::default().fg(TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  d              ", Style::default().fg(ACCENT)),
            Span::styled("  delete worktree", Style::default().fg(TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  s              ", Style::default().fg(ACCENT)),
            Span::styled("  sync worktree", Style::default().fg(TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  r              ", Style::default().fg(ACCENT)),
            Span::styled("  refresh all", Style::default().fg(TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  ?              ", Style::default().fg(ACCENT)),
            Span::styled("  toggle help", Style::default().fg(TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  q / Esc        ", Style::default().fg(ACCENT)),
            Span::styled("  quit", Style::default().fg(TEXT)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  press any key to close",
            Style::default().fg(MUTED),
        )),
    ];

    let block = Block::default()
        .title(Span::styled(
            " Keyboard Shortcuts ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT))
        .style(Style::default().bg(BG));

    f.render_widget(Paragraph::new(help_text).block(block), popup);
}

fn draw_input_overlay(f: &mut Frame, area: Rect, prompt: &str, buf: &str) {
    let popup = centered_rect(50, 20, area);
    f.render_widget(Clear, popup);

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(format!("  {}", prompt), Style::default().fg(TEXT)),
            Span::styled(buf, Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
            Span::styled("_", Style::default().fg(ACCENT)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Enter to confirm, Esc to cancel",
            Style::default().fg(MUTED),
        )),
    ];

    let block = Block::default()
        .title(Span::styled(
            " Input ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT))
        .style(Style::default().bg(BG));

    f.render_widget(Paragraph::new(lines).block(block), popup);
}

fn draw_confirm_overlay(f: &mut Frame, area: Rect, msg: &str) {
    let popup = centered_rect(50, 20, area);
    f.render_widget(Clear, popup);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {}", msg),
            Style::default().fg(TEXT),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  y", Style::default().fg(ACCENT)),
            Span::styled(" confirm  ", Style::default().fg(MUTED)),
            Span::styled("any other key", Style::default().fg(ACCENT)),
            Span::styled(" cancel", Style::default().fg(MUTED)),
        ]),
    ];

    let block = Block::default()
        .title(Span::styled(
            " Confirm ",
            Style::default().fg(MODIFIED).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(MODIFIED))
        .style(Style::default().bg(BG));

    f.render_widget(Paragraph::new(lines).block(block), popup);
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn panel_block(panel: Panel, active: bool) -> Block<'static> {
    let border_color = if active { ACCENT } else { DIM };
    let title_color = if active { ACCENT } else { MUTED };
    Block::default()
        .title(Span::styled(
            panel.title(),
            Style::default().fg(title_color),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{}~", truncated)
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
