use std::io::{stdout, Stdout};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame, Terminal,
};

use crate::{fleet::FleetState, git};

// ── Palette ──────────────────────────────────────────────────────────────

const ACCENT: Color = Color::Rgb(212, 255, 0);
const RUNNING: Color = Color::Rgb(134, 239, 172);
const MODIFIED: Color = Color::Rgb(251, 191, 36);
const MUTED: Color = Color::Rgb(80, 80, 80);
const DIM: Color = Color::Rgb(38, 38, 38);
const TEXT: Color = Color::Rgb(220, 220, 220);
const SEL_BG: Color = Color::Rgb(20, 20, 20);
const BRANCH_COLOR: Color = Color::Rgb(175, 175, 175);
const TASK_COLOR: Color = Color::Rgb(110, 110, 110);

// ── Data model ───────────────────────────────────────────────────────────

struct TaskRow {
    branch: String,
    task: String,
    exists: bool,
    is_dirty: bool,
    last_commit: Option<String>,
    agent_pid: Option<u32>,
    path: String,
}

// ── Agent detection ──────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn detect_agent(wt_path: &std::path::Path, agent: &str) -> Option<u32> {
    let procs = std::fs::read_dir("/proc").ok()?;
    for entry in procs.flatten() {
        let fname = entry.file_name();
        let pid_str = fname.to_string_lossy();
        if !pid_str.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let cmdline_path = entry.path().join("cmdline");
        let Ok(bytes) = std::fs::read(&cmdline_path) else {
            continue;
        };
        // cmdline args are null-separated; check if any contains agent name
        let cmdline = String::from_utf8_lossy(&bytes).to_lowercase();
        if !cmdline.contains(agent) {
            continue;
        }
        let cwd_link = entry.path().join("cwd");
        let Ok(cwd) = std::fs::read_link(&cwd_link) else {
            continue;
        };
        if cwd == wt_path {
            return pid_str.parse().ok();
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn detect_agent(wt_path: &std::path::Path, agent: &str) -> Option<u32> {
    let pgrep = std::process::Command::new("pgrep")
        .args(["-f", agent])
        .output()
        .ok()?;
    if !pgrep.status.success() {
        return None;
    }
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
    None
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn detect_agent(_wt_path: &std::path::Path, _agent: &str) -> Option<u32> {
    None
}

fn fetch_rows(state: &FleetState) -> Vec<TaskRow> {
    state
        .tasks
        .iter()
        .map(|ft| {
            let wt_path = PathBuf::from(&ft.path);
            let exists = wt_path.exists();
            let is_dirty = exists && git::is_dirty(&wt_path).unwrap_or(false);
            let last_commit = if exists {
                git::last_commit_relative(&wt_path)
            } else {
                None
            };
            let agent_pid = if exists {
                detect_agent(&wt_path, &state.agent)
            } else {
                None
            };
            TaskRow {
                branch: ft.branch.clone(),
                task: ft.task.clone(),
                path: ft.path.clone(),
                exists,
                is_dirty,
                last_commit,
                agent_pid,
            }
        })
        .collect()
}

// ── App state ────────────────────────────────────────────────────────────

struct App {
    state: FleetState,
    rows: Vec<TaskRow>,
    selected: usize,
    last_refresh: Instant,
    table_state: TableState,
}

impl App {
    fn new(state: FleetState) -> Self {
        let rows = fetch_rows(&state);
        let mut table_state = TableState::default();
        if !rows.is_empty() {
            table_state.select(Some(0));
        }
        Self {
            state,
            rows,
            selected: 0,
            last_refresh: Instant::now(),
            table_state,
        }
    }

    fn refresh(&mut self) {
        self.rows = fetch_rows(&self.state);
        self.last_refresh = Instant::now();
        if !self.rows.is_empty() {
            self.selected = self.selected.min(self.rows.len() - 1);
        }
        self.table_state.select(if self.rows.is_empty() {
            None
        } else {
            Some(self.selected)
        });
    }

    fn select_prev(&mut self) {
        if self.rows.is_empty() {
            return;
        }
        self.selected = self.selected.saturating_sub(1);
        self.table_state.select(Some(self.selected));
    }

    fn select_next(&mut self) {
        if self.rows.is_empty() {
            return;
        }
        self.selected = (self.selected + 1).min(self.rows.len() - 1);
        self.table_state.select(Some(self.selected));
    }
}

// ── Entry point ──────────────────────────────────────────────────────────

pub fn run_tui(fleet_state: FleetState) -> Result<()> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;

    let result = run_loop(&mut terminal, fleet_state);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    fleet_state: FleetState,
) -> Result<()> {
    let mut app = App::new(fleet_state);
    const TICK: Duration = Duration::from_millis(100);
    const REFRESH: Duration = Duration::from_secs(2);

    loop {
        terminal.draw(|f| draw(f, &mut app))?;

        if event::poll(TICK)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Up | KeyCode::Char('k') => app.select_prev(),
                        KeyCode::Down | KeyCode::Char('j') => app.select_next(),
                        KeyCode::Char('r') => app.refresh(),
                        _ => {}
                    }
                }
            }
        }

        if app.last_refresh.elapsed() >= REFRESH {
            app.refresh();
        }
    }
    Ok(())
}

// ── Rendering ────────────────────────────────────────────────────────────

fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();

    // Fill background
    f.render_widget(
        Block::default().style(Style::default().bg(Color::Rgb(8, 8, 8))),
        area,
    );

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // header
            Constraint::Min(5),    // table
            Constraint::Length(9), // detail panel
            Constraint::Length(1), // footer
        ])
        .split(area);

    draw_header(f, app, chunks[0]);
    draw_table(f, app, chunks[1]);
    draw_detail(f, app, chunks[2]);
    draw_footer(f, app, chunks[3]);
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let running = app.rows.iter().filter(|r| r.agent_pid.is_some()).count();
    let total = app.rows.len();

    // Split header: info left, hints right
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(20), Constraint::Length(34)])
        .split(area);

    // Left: fleet info
    let info = Line::from(vec![
        Span::styled(" workz ", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::styled("fleet  ", Style::default().fg(MUTED)),
        Span::styled(
            format!("{} task{}", total, if total == 1 { "" } else { "s" }),
            Style::default().fg(TEXT),
        ),
        Span::styled("  ·  ", Style::default().fg(DIM)),
        Span::styled(app.state.agent.clone(), Style::default().fg(RUNNING)),
        if running > 0 {
            Span::styled(
                format!("  ·  {} agent{} running", running, if running == 1 { "" } else { "s" }),
                Style::default().fg(RUNNING),
            )
        } else {
            Span::raw("")
        },
    ]);

    // Right: key hints
    let hints = Line::from(vec![
        Span::styled("↑↓", Style::default().fg(MUTED)),
        Span::styled(" nav  ", Style::default().fg(DIM)),
        Span::styled("r", Style::default().fg(MUTED)),
        Span::styled(" refresh  ", Style::default().fg(DIM)),
        Span::styled("q", Style::default().fg(MUTED)),
        Span::styled(" quit ", Style::default().fg(DIM)),
    ]);

    let border = Style::default().fg(DIM);
    f.render_widget(
        Paragraph::new(info).block(Block::default().borders(Borders::BOTTOM).border_style(border)),
        cols[0],
    );
    f.render_widget(
        Paragraph::new(hints)
            .alignment(ratatui::layout::Alignment::Right)
            .block(Block::default().borders(Borders::BOTTOM).border_style(border)),
        cols[1],
    );
}

fn draw_table(f: &mut Frame, app: &mut App, area: Rect) {
    let header_style = Style::default()
        .fg(Color::Rgb(55, 55, 55))
        .add_modifier(Modifier::BOLD);

    let header = Row::new(vec![
        Cell::from("  branch").style(header_style),
        Cell::from("task").style(header_style),
        Cell::from("status").style(header_style),
        Cell::from("commit").style(header_style),
    ])
    .height(1);

    let rows: Vec<Row> = app
        .rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let selected = i == app.selected;

            let branch_cell = Cell::from(format!("  {}", truncate(&row.branch, 34))).style(
                if selected {
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(BRANCH_COLOR)
                },
            );

            let task_cell = Cell::from(truncate(&row.task, 26))
                .style(Style::default().fg(TASK_COLOR));

            let (status_str, status_color) = if !row.exists {
                ("  missing", Color::Rgb(239, 68, 68))
            } else if row.agent_pid.is_some() {
                ("  ● running", RUNNING)
            } else if row.is_dirty {
                ("  modified", MODIFIED)
            } else {
                ("  clean", MUTED)
            };
            let status_cell = Cell::from(status_str)
                .style(Style::default().fg(status_color));

            let commit_str = row.last_commit.as_deref().unwrap_or("—");
            let commit_cell = Cell::from(truncate(commit_str, 14))
                .style(Style::default().fg(Color::Rgb(60, 60, 60)));

            Row::new(vec![branch_cell, task_cell, status_cell, commit_cell])
                .style(if selected {
                    Style::default().bg(SEL_BG)
                } else {
                    Style::default()
                })
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Min(30),
            Constraint::Min(22),
            Constraint::Length(14),
            Constraint::Length(14),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(DIM)),
    )
    .highlight_style(Style::default().bg(SEL_BG))
    .highlight_symbol("");

    f.render_stateful_widget(table, area, &mut app.table_state);
}

fn draw_detail(f: &mut Frame, app: &App, area: Rect) {
    let border_style = Style::default().fg(DIM);
    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(border_style);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.rows.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled("  no tasks", Style::default().fg(MUTED)))),
            inner,
        );
        return;
    }

    let row = &app.rows[app.selected.min(app.rows.len() - 1)];

    let label_style = Style::default().fg(Color::Rgb(55, 55, 55));
    let value_style = Style::default().fg(Color::Rgb(140, 140, 140));
    let path_style = Style::default().fg(Color::Rgb(70, 70, 70));

    let status_span = if !row.exists {
        Span::styled("missing", Style::default().fg(Color::Rgb(239, 68, 68)))
    } else if row.is_dirty {
        Span::styled("modified", Style::default().fg(MODIFIED))
    } else {
        Span::styled("clean", Style::default().fg(RUNNING))
    };

    let agent_line = if let Some(pid) = row.agent_pid {
        Line::from(vec![
            Span::styled("  agent   ", label_style),
            Span::styled(app.state.agent.clone(), Style::default().fg(TEXT)),
            Span::styled("  ·  pid ", label_style),
            Span::styled(
                pid.to_string(),
                Style::default().fg(Color::Rgb(147, 197, 253)),
            ),
            Span::styled("  ·  ", label_style),
            Span::styled("● running", Style::default().fg(RUNNING)),
        ])
    } else {
        Line::from(vec![
            Span::styled("  agent   ", label_style),
            Span::styled("not detected", Style::default().fg(MUTED)),
        ])
    };

    let lines = vec![
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                row.branch.clone(),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled("  task    ", label_style),
            Span::styled(row.task.clone(), value_style),
        ]),
        Line::from(vec![
            Span::styled("  path    ", label_style),
            Span::styled(row.path.clone(), path_style),
        ]),
        Line::from(vec![
            Span::styled("  status  ", label_style),
            status_span,
        ]),
        Line::from(vec![
            Span::styled("  commit  ", label_style),
            Span::styled(
                row.last_commit.clone().unwrap_or_else(|| "—".into()),
                value_style,
            ),
        ]),
        agent_line,
    ];

    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let elapsed = app.last_refresh.elapsed().as_secs();
    let refresh_str = if elapsed == 0 {
        "just now".to_string()
    } else {
        format!("{}s ago", elapsed)
    };

    let line = Line::from(vec![
        Span::styled("  ↻ auto-refresh every 2s  ·  last: ", Style::default().fg(DIM)),
        Span::styled(refresh_str, Style::default().fg(Color::Rgb(55, 55, 55))),
    ]);

    f.render_widget(Paragraph::new(line), area);
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{}…", truncated)
    }
}
