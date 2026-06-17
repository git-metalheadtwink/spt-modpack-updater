use std::collections::VecDeque;
use std::io::{stdout, Stdout};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame, Terminal,
};

use crate::progress::ProgressEvent;
use crate::types::UpdateStatus;

// ── palette ───────────────────────────────────────────────────────────────────

const PINK:   Color = Color::Rgb(255, 100, 255);
const LBLUE:  Color = Color::Rgb(130, 210, 255);
const DIM:    Color = Color::DarkGray;
const BORDER: Color = Color::Rgb(105, 105, 105);

// ── public config ─────────────────────────────────────────────────────────────

pub struct Config {
    pub game_path:      PathBuf,
    pub spt_version:    Option<String>,
    pub initial_branch: String,
}

// ── state ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Mode {
    Idle,
    Running { is_user_op: bool },
    SelectBranch { branches: Vec<String>, sel: usize },
    Done(bool),
    SelfUpdatePrompt { version: String, url: String },
}

struct App {
    game_path:   PathBuf,
    branch:      String,
    spt_version: Option<String>,
    status:      UpdateStatus,
    menu_sel:    usize,
    mode:        Mode,
    op_name:     String,
    progress:    f64,
    phase:       String,
    log:         VecDeque<String>,
    tick:                  u64,
    confirm_quit:          bool,
    pending_self_update:   Option<(String, String)>,
    self_update_in_progress: bool,
    rx:                    Receiver<ProgressEvent>,
    tx:                    Sender<ProgressEvent>,
}

impl App {
    fn new(cfg: Config, rx: Receiver<ProgressEvent>, tx: Sender<ProgressEvent>) -> Self {
        Self {
            game_path:   cfg.game_path,
            branch:      cfg.initial_branch,
            spt_version: cfg.spt_version,
            status:      UpdateStatus::NotInitialized,
            menu_sel:    0,
            mode:        Mode::Running { is_user_op: false },
            op_name:     String::new(),
            progress:    0.0,
            phase:       "Checking for updates...".into(),
            log:         VecDeque::new(),
            tick:                    0,
            confirm_quit:            false,
            pending_self_update:     None,
            self_update_in_progress: false,
            rx,
            tx,
        }
    }

    fn log_push(&mut self, s: String) {
        self.log.push_back(s);
        while self.log.len() > 500 { self.log.pop_front(); }
    }

    fn drain_events(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                ProgressEvent::Phase { name, current, total } => {
                    let frac = if total > 0 { current as f64 / total as f64 } else { 0.0 };
                    let (lo, hi) = crate::git::phase_range(&name);
                    self.progress = (lo + (hi - lo) * frac) / 100.0;
                    self.phase = format!("{} ({}/{})", name, current, total);
                }
                ProgressEvent::Log(m) => { self.log_push(m); }
                ProgressEvent::StatusResult(st) => { self.status = st; }
                ProgressEvent::BranchList(bs) => {
                    let sel = bs.iter().position(|b| b == &self.branch).unwrap_or(0);
                    self.mode = Mode::SelectBranch { branches: bs, sel };
                }
                ProgressEvent::Done => {
                    self.progress = 1.0;
                    if self.self_update_in_progress {
                        // New exe is in place — relaunch diverges, never returns.
                        crate::updater::relaunch();
                    }
                    self.mode = match &self.mode {
                        Mode::Running { is_user_op: true } => Mode::Done(true),
                        _ => Mode::Idle,
                    };
                }
                ProgressEvent::Error(e) => {
                    self.log_push(format!("Error: {}", e));
                    self.self_update_in_progress = false;
                    if matches!(self.mode, Mode::Running { is_user_op: true }) {
                        self.mode = Mode::Done(false);
                    } else {
                        self.status = UpdateStatus::Error(e);
                        self.mode = Mode::Idle;
                    }
                }
                ProgressEvent::SelfUpdateAvailable { version, url } => {
                    self.pending_self_update = Some((version, url));
                }
            }
        }

        // Show the self-update prompt as soon as we become idle.
        if self.mode == Mode::Idle {
            if let Some((version, url)) = self.pending_self_update.take() {
                self.mode = Mode::SelfUpdatePrompt { version, url };
            }
        }
    }

    fn handle_key(&mut self, code: KeyCode) -> bool {
        // Confirm-quit dialog intercepts all keys when active
        if self.confirm_quit {
            match code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => return true,
                _ => { self.confirm_quit = false; }
            }
            return false;
        }
        match self.mode.clone() {
            Mode::Idle => match code {
                KeyCode::Up    => { self.menu_sel = self.menu_sel.saturating_sub(1); }
                KeyCode::Down  => { if self.menu_sel < 2 { self.menu_sel += 1; } }
                KeyCode::Enter => { self.start_action(); }
                KeyCode::Esc   => return true,
                _ => {}
            },
            Mode::Running { is_user_op } => {
                if code == KeyCode::Esc {
                    if is_user_op {
                        // Don't quit mid-operation — ask first
                        self.confirm_quit = true;
                    } else {
                        return true;
                    }
                }
            }
            Mode::SelectBranch { branches, mut sel } => {
                match code {
                    KeyCode::Up    => { sel = sel.saturating_sub(1); }
                    KeyCode::Down  => { if sel + 1 < branches.len() { sel += 1; } }
                    KeyCode::Enter => { let b = branches[sel].clone(); self.switch_branch(b); return false; }
                    KeyCode::Esc   => { self.mode = Mode::Idle; return false; }
                    _ => {}
                }
                self.mode = Mode::SelectBranch { branches, sel };
            }
            Mode::Done(_) => match code {
                KeyCode::Enter => {
                    self.mode = Mode::Idle;
                    self.log.clear();
                    self.progress = 0.0;
                    self.phase = String::new();
                }
                KeyCode::Esc => return true,
                _ => {}
            },
            Mode::SelfUpdatePrompt { url, .. } => {
                let url = url.clone();
                match code {
                    KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                        self.start_self_update(url);
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        self.mode = Mode::Idle;
                    }
                    _ => {}
                }
            }
        }
        false
    }

    fn start_action(&mut self) {
        self.progress = 0.0;
        self.log.clear();
        let tx   = self.tx.clone();
        let path = self.game_path.clone();
        let br   = self.branch.clone();
        match self.menu_sel {
            0 => {
                self.op_name = "UPDATE".into();
                self.phase   = "Starting update...".into();
                self.mode    = Mode::Running { is_user_op: true };
                std::thread::spawn(move || crate::git::run_update(&path, &br, &tx));
            }
            1 => {
                self.op_name = "REPAIR".into();
                self.phase   = "Repairing config files...".into();
                self.mode    = Mode::Running { is_user_op: true };
                std::thread::spawn(move || crate::git::run_repair(&path, &tx));
            }
            _ => {
                self.phase = "Fetching branch list...".into();
                self.mode  = Mode::Running { is_user_op: false };
                std::thread::spawn(move || crate::git::run_fetch_branches(&path, &tx));
            }
        }
    }

    fn switch_branch(&mut self, new_branch: String) {
        self.branch = new_branch.clone();
        let _ = std::fs::write(self.game_path.join(".updater-branch"), &new_branch);
        let tx   = self.tx.clone();
        let path = self.game_path.clone();
        self.progress = 0.0;
        self.log.clear();
        self.phase = "Checking for updates...".into();
        self.mode  = Mode::Running { is_user_op: false };
        std::thread::spawn(move || crate::git::run_check_update(&path, &new_branch, &tx));
    }

    fn start_self_update(&mut self, url: String) {
        self.op_name                 = "SELF-UPDATE".into();
        self.phase                   = "Downloading...".into();
        self.progress                = 0.0;
        self.log.clear();
        self.mode                    = Mode::Running { is_user_op: true };
        self.self_update_in_progress = true;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            if let Err(e) = crate::updater::download_and_replace(&url, &tx) {
                let _ = tx.send(ProgressEvent::Error(e.to_string()));
            }
        });
    }
}

// ── layout helpers ────────────────────────────────────────────────────────────

fn centered_rect(pct_x: u16, pct_y: u16, area: Rect) -> Rect {
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - pct_y) / 2),
            Constraint::Percentage(pct_y),
            Constraint::Percentage((100 - pct_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - pct_x) / 2),
            Constraint::Percentage(pct_x),
            Constraint::Percentage((100 - pct_x) / 2),
        ])
        .split(vert[1])[1]
}

fn hline(f: &mut Frame, area: Rect, color: Color) {
    let s: String = std::iter::repeat('─').take(area.width as usize).collect();
    f.render_widget(Paragraph::new(Span::styled(s, Style::default().fg(color))), area);
}

fn status_display(status: &UpdateStatus) -> (Color, String) {
    match status {
        UpdateStatus::UpToDate { hash } =>
            (Color::Green,  format!("◆ Up to date ({})", &hash[..hash.len().min(8)])),
        UpdateStatus::Available { local, remote } =>
            (Color::Yellow, format!("◆ Update available  {} → {}",
                &local[..local.len().min(8)], &remote[..remote.len().min(8)])),
        UpdateStatus::NotInitialized =>
            (LBLUE,         "◆ Not installed — UPDATE to install".into()),
        UpdateStatus::Error(e) =>
            (Color::Red,    format!("◆ Error: {}", e)),
    }
}

// ── main TUI (always visible; dims when popup is shown) ───────────────────────

fn draw_main(f: &mut Frame, app: &App, dimmed: bool) {
    let area = f.area();

    let border_c = if dimmed { DIM } else { BORDER };
    let text_c   = if dimmed { DIM } else { Color::White };
    let info_c   = if dimmed { DIM } else { LBLUE };
    let accent_c = if dimmed { DIM } else { PINK };

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_c))
        .title_top(
            Line::from(vec![Span::styled(
                " SPT MODPACK UPDATER ",
                Style::default().fg(border_c),
            )])
            .alignment(Alignment::Center),
        )
        .title_bottom(
            Line::from(vec![Span::styled(
                format!(" v{} ", env!("CARGO_PKG_VERSION")),
                Style::default().fg(border_c),
            )])
            .alignment(Alignment::Right),
        );

    let inner = outer.inner(area);
    f.render_widget(outer, area);

    if inner.height < 10 || inner.width < 30 {
        f.render_widget(Paragraph::new("Terminal too small"), inner);
        return;
    }

    let secs = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // [0] info row
            Constraint::Length(1), // [1] separator
            Constraint::Length(1), // [2] blank
            Constraint::Length(3), // [3] menu / branch picker
            Constraint::Length(1), // [4] blank
            Constraint::Min(0),    // [5] spacer — pushes hint to bottom
            Constraint::Length(1), // [6] bottom hint
        ])
        .split(inner);

    // [0] info row
    {
        let (sc, st) = status_display(&app.status);
        let sc = if dimmed { DIM } else { sc };
        let mut spans = vec![
            Span::raw("  "),
            Span::styled("◆ ", Style::default().fg(accent_c)),
            Span::styled("Branch: ", Style::default().fg(text_c)),
            Span::styled(app.branch.clone(), Style::default().fg(info_c)),
        ];
        if let Some(v) = &app.spt_version {
            spans.push(Span::raw("   "));
            spans.push(Span::styled(format!("SPT {}", v), Style::default().fg(DIM)));
        }
        spans.push(Span::raw("   "));
        spans.push(Span::styled(st, Style::default().fg(sc)));
        f.render_widget(Paragraph::new(Line::from(spans)), secs[0]);
    }

    // [1] separator
    hline(f, secs[1], border_c);

    // [3] menu / branch picker
    {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(secs[3]);

        match &app.mode {
            Mode::SelectBranch { branches, sel } => {
                let n      = branches.len();
                let scroll = sel.saturating_sub(2);
                for (i, row) in rows.iter().enumerate() {
                    let idx    = scroll + i;
                    if idx >= n { break; }
                    let b      = &branches[idx];
                    let is_sel = idx == *sel;
                    let is_cur = b == &app.branch;
                    let suffix = if is_cur { "  (current)" } else { "" };
                    let label  = if is_sel { format!("  > {}{}", b, suffix) }
                                 else      { format!("    {}{}", b, suffix) };
                    let style  = if is_sel {
                        Style::default().bg(PINK).fg(Color::Black).add_modifier(Modifier::BOLD)
                    } else if is_cur {
                        Style::default().fg(Color::Green)
                    } else {
                        Style::default().fg(text_c)
                    };
                    let padded = format!("{:<width$}", label, width = row.width as usize);
                    f.render_widget(Paragraph::new(Span::styled(padded, style)), *row);
                }
            }
            _ => {
                // Hide menu during background startup check to avoid an unresponsive flash
                if matches!(app.mode, Mode::Running { is_user_op: false }) {
                    const SPIN: [char; 4] = ['|', '/', '-', '\\'];
                    let ch = SPIN[(app.tick / 3) as usize % SPIN.len()];
                    f.render_widget(
                        Paragraph::new(Span::styled(
                            format!("  {} Checking...", ch),
                            Style::default().fg(DIM),
                        )),
                        rows[0],
                    );
                } else {
                    let opts      = ["UPDATE", "REPAIR", "BRANCH"];
                    let is_active = matches!(app.mode, Mode::Idle) && !dimmed;
                    for (i, (opt, row)) in opts.iter().zip(rows.iter()).enumerate() {
                        let is_sel = is_active && i == app.menu_sel;
                        let label  = if is_sel { format!("  > {}", opt) }
                                     else      { format!("    {}", opt) };
                        let style  = if is_sel {
                            Style::default().bg(PINK).fg(Color::Black).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(text_c)
                        };
                        let padded = format!("{:<width$}", label, width = row.width as usize);
                        f.render_widget(Paragraph::new(Span::styled(padded, style)), *row);
                    }
                }
            }
        }
    }

    // [6] bottom hint line
    {
        let text = if matches!(app.mode, Mode::SelfUpdatePrompt { .. }) {
            ""
        } else if dimmed {
            "  (operation in progress)"
        } else {
            match &app.mode {
                Mode::Idle                          => "  ↑↓ Navigate   Enter Select   Esc Quit",
                Mode::Running { is_user_op: false } => "  Esc to quit",
                Mode::SelectBranch { .. }           => "  ↑↓ Navigate   Enter Select   Esc Cancel",
                _ => "",
            }
        };
        f.render_widget(
            Paragraph::new(Span::styled(text, Style::default().fg(DIM))),
            secs[6],
        );
    }
}

// ── popup overlay ─────────────────────────────────────────────────────────────

fn draw_popup(f: &mut Frame, app: &App) {
    let area       = f.area();
    let popup_area = centered_rect(72, 68, area);

    // Erase what the dimmed main TUI drew in this region
    f.render_widget(Clear, popup_area);

    let done    = matches!(app.mode, Mode::Done(_));
    let success = matches!(app.mode, Mode::Done(true));
    let accent  = if done { if success { Color::Green } else { Color::Red } } else { PINK };

    let border = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(accent))
        .title(Span::styled(
            format!(" {} ", app.op_name),
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ))
        .title_alignment(Alignment::Center);

    let inner = border.inner(popup_area);
    f.render_widget(border, popup_area);

    if inner.height < 5 { return; }

    let constraints: Vec<Constraint> = if done {
        vec![
            Constraint::Length(1), // progress bar
            Constraint::Length(1), // phase / result text
            Constraint::Min(1),    // log
            Constraint::Length(1), // separator
            Constraint::Length(1), // button row
        ]
    } else {
        vec![
            Constraint::Length(1), // progress bar
            Constraint::Length(1), // phase text
            Constraint::Min(1),    // log
            Constraint::Length(1), // spinner line
        ]
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    // Progress bar
    {
        let w      = chunks[0].width as usize;
        let bar_w  = w.saturating_sub(8).max(2);
        let filled = ((app.progress * bar_w as f64) as usize).min(bar_w);
        let empty  = bar_w - filled;
        let pct    = (app.progress * 100.0).min(100.0) as u32;

        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::raw(" "),
                Span::styled("█".repeat(filled), Style::default().fg(accent)),
                Span::styled("░".repeat(empty),  Style::default().fg(DIM)),
                Span::styled(format!("  {:>3}%", pct), Style::default().fg(Color::White)),
            ])),
            chunks[0],
        );
    }

    // Phase / result line
    {
        let (text, color) = if done {
            if success { (" ✓ Completed successfully".into(), Color::Green) }
            else       { (" ✗ Failed — see log below".into(), Color::Red)   }
        } else {
            (format!(" {}", app.phase), DIM)
        };
        f.render_widget(
            Paragraph::new(Span::styled(text, Style::default().fg(color))),
            chunks[1],
        );
    }

    // Scrolling log
    {
        let visible = chunks[2].height as usize;
        let skip    = app.log.len().saturating_sub(visible);
        let items: Vec<ListItem> = app.log.iter().skip(skip).map(|l| {
            let color = if l.starts_with("Error") || l.starts_with('!') { Color::Red }
                        else if l.starts_with("Protected") || l.starts_with("Restored") { Color::Green }
                        else { DIM };
            ListItem::new(Span::styled(format!(" {}", l), Style::default().fg(color)))
        }).collect();
        f.render_widget(List::new(items), chunks[2]);
    }

    // Spinner line (only while running)
    if !done {
        const SPIN: [char; 8] = ['|', '/', '-', '\\', '|', '/', '-', '\\'];
        let ch = SPIN[(app.tick / 3) as usize % SPIN.len()];
        f.render_widget(
            Paragraph::new(Span::styled(
                format!(" {} ", ch),
                Style::default().fg(PINK),
            )),
            chunks[3],
        );
    }

    // Separator + button row (only when done)
    if done {
        hline(f, chunks[3], DIM);

        let label  = if success { " ✓  Press Enter to continue " }
                     else       { " ✗  Press Enter to continue " };
        let btn_w  = label.len() as u16;
        let btn_x  = chunks[4].x + (chunks[4].width.saturating_sub(btn_w)) / 2;
        let btn_rect = Rect {
            x: btn_x,
            y: chunks[4].y,
            width: btn_w.min(chunks[4].width),
            height: 1,
        };

        f.render_widget(
            Paragraph::new(Span::styled(
                label,
                Style::default().bg(accent).fg(Color::Black).add_modifier(Modifier::BOLD),
            )),
            btn_rect,
        );

        // "Esc Quit" at far right of the same row
        let esc      = "Esc Quit ";
        let esc_x    = chunks[4].x + chunks[4].width.saturating_sub(esc.len() as u16);
        let esc_rect = Rect { x: esc_x, y: chunks[4].y, width: esc.len() as u16, height: 1 };
        f.render_widget(
            Paragraph::new(Span::styled(esc, Style::default().fg(DIM))),
            esc_rect,
        );
    }
}

// ── self-update prompt ────────────────────────────────────────────────────────

fn draw_self_update_prompt(f: &mut Frame, app: &App) {
    let Mode::SelfUpdatePrompt { version, .. } = &app.mode else { return };

    let area = f.area();
    let w = 58u16.min(area.width);
    let h = 8u16.min(area.height);
    let popup = Rect {
        x:      area.x + (area.width.saturating_sub(w)) / 2,
        y:      area.y + (area.height.saturating_sub(h)) / 2,
        width:  w,
        height: h,
    };

    f.render_widget(Clear, popup);

    let border = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(PINK))
        .title(Span::styled(
            " Update Available ",
            Style::default().fg(PINK).add_modifier(Modifier::BOLD),
        ))
        .title_alignment(Alignment::Center);

    let inner = border.inner(popup);
    f.render_widget(border, popup);

    if inner.height < 4 { return; }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // blank
            Constraint::Length(1), // new version
            Constraint::Length(1), // current version
            Constraint::Min(1),    // spacer
            Constraint::Length(1), // buttons
        ])
        .split(inner);

    f.render_widget(
        Paragraph::new(Span::styled(
            format!("  New version v{} is available!", version),
            Style::default().fg(Color::White),
        )),
        rows[1],
    );

    f.render_widget(
        Paragraph::new(Span::styled(
            format!("  Currently running v{}", env!("CARGO_PKG_VERSION")),
            Style::default().fg(DIM),
        )),
        rows[2],
    );

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " [ Y ] Update & relaunch ",
                Style::default().bg(PINK).fg(Color::Black).add_modifier(Modifier::BOLD),
            ),
            Span::raw("   "),
            Span::styled(
                " [ N ] Skip ",
                Style::default().bg(DIM).fg(Color::Black).add_modifier(Modifier::BOLD),
            ),
        ]))
        .alignment(Alignment::Center),
        rows[4],
    );
}

// ── confirm-quit dialog ───────────────────────────────────────────────────────

fn draw_confirm(f: &mut Frame) {
    let area = f.area();
    let w    = 52u16.min(area.width);
    let h    = 6u16.min(area.height);
    let popup = Rect {
        x:      area.x + (area.width.saturating_sub(w)) / 2,
        y:      area.y + (area.height.saturating_sub(h)) / 2,
        width:  w,
        height: h,
    };

    f.render_widget(Clear, popup);

    let border = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .title(Span::styled(
            " Quit? ",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ))
        .title_alignment(Alignment::Center);

    let inner = border.inner(popup);
    f.render_widget(border, popup);

    if inner.height < 2 { return; }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);

    f.render_widget(
        Paragraph::new(Span::styled(
            "Operation in progress. Quit anyway?",
            Style::default().fg(Color::White),
        )).alignment(Alignment::Center),
        rows[0],
    );

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " [ Y ] Yes, quit ",
                Style::default().bg(Color::Red).fg(Color::White).add_modifier(Modifier::BOLD),
            ),
            Span::raw("   "),
            Span::styled(
                " [ N ] Cancel ",
                Style::default().bg(DIM).fg(Color::Black).add_modifier(Modifier::BOLD),
            ),
        ])).alignment(Alignment::Center),
        rows[1],
    );
}

// ── top-level draw ────────────────────────────────────────────────────────────

fn ui(f: &mut Frame, app: &App) {
    let show_popup          = matches!(app.mode, Mode::Running { is_user_op: true } | Mode::Done(_));
    let show_update_prompt  = matches!(app.mode, Mode::SelfUpdatePrompt { .. });
    let dimmed              = show_popup || show_update_prompt;
    draw_main(f, app, dimmed);
    if show_popup {
        draw_popup(f, app);
    }
    if show_update_prompt {
        draw_self_update_prompt(f, app);
    }
    if app.confirm_quit {
        draw_confirm(f);
    }
}

// ── entry point ───────────────────────────────────────────────────────────────

pub fn run(cfg: Config) -> anyhow::Result<()> {
    let (tx, rx) = mpsc::channel::<ProgressEvent>();
    let mut app  = App::new(cfg, rx, tx.clone());

    {
        let tx2 = tx.clone();
        std::thread::spawn(move || crate::updater::check_self_update(&tx2));
    }

    {
        let path   = app.game_path.clone();
        let branch = app.branch.clone();
        let tx2    = tx.clone();
        std::thread::spawn(move || crate::git::run_check_update(&path, &branch, &tx2));
    }

    terminal::enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen, cursor::Hide)?;

    let backend  = CrosstermBackend::new(out);
    let mut term = Terminal::new(backend)?;

    let result = event_loop(&mut term, &mut app);

    let _ = terminal::disable_raw_mode();
    let _ = execute!(term.backend_mut(), LeaveAlternateScreen, cursor::Show);

    result
}

fn event_loop(
    term: &mut Terminal<CrosstermBackend<Stdout>>,
    app:  &mut App,
) -> anyhow::Result<()> {
    loop {
        app.drain_events();
        app.tick = app.tick.wrapping_add(1);
        term.draw(|f| ui(f, app))?;

        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    if app.handle_key(k.code) { break; }
                }
                Event::Resize(_, _) => { term.clear()?; }
                _ => {}
            }
        }
    }
    Ok(())
}
