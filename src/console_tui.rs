//! Full-screen terminal console shell for CoreRoom snapshots.
//!
//! v0.9 starts with an explicit, read-only shell. It renders an already-built
//! [`CoreRoomSnapshot`](crate::console_snapshot::CoreRoomSnapshot) and does not
//! derive state from chat prose or mutate project files.

use std::fs;
use std::io::{self, IsTerminal as _, Write};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::{CrosstermBackend, TestBackend};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use crate::console_conversation::build_public_conversation;
use crate::console_health::overview_health_signals;
use crate::console_layout::{compute_console_layout, RightRailSection};
use crate::console_overview::{build_console_overview, ConsoleOverview, OverviewPulse};
use crate::console_snapshot::{
    CoreRoomSnapshot, DirtyState, HealthSeverity, StatusState, WorkLifecycle,
};

/// Load and validate a TOML-encoded CoreRoom console snapshot.
pub fn load_snapshot(path: &Path) -> Result<CoreRoomSnapshot> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("read console snapshot {}", path.display()))?;
    let snapshot: CoreRoomSnapshot = toml::from_str(&content)
        .with_context(|| format!("parse console snapshot {}", path.display()))?;
    snapshot
        .validate()
        .with_context(|| format!("validate console snapshot {}", path.display()))?;
    Ok(snapshot)
}

/// Run the interactive read-only full-screen console for a snapshot file.
pub fn run_snapshot_console(snapshot_path: &Path) -> Result<()> {
    let snapshot = load_snapshot(snapshot_path)?;
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        anyhow::bail!("cr console requires an interactive TTY");
    }

    let _guard = ConsoleTerminalGuard::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend).context("create console terminal")?;
    terminal.clear().context("clear console terminal")?;

    loop {
        terminal
            .draw(|frame| render_console_frame(frame, &snapshot))
            .context("render console frame")?;
        if event::poll(Duration::from_millis(200)).context("poll console input")? {
            match event::read().context("read console input")? {
                Event::Key(key) if key.kind == KeyEventKind::Press && is_exit_key(key.code) => {
                    break;
                }
                _ => {}
            }
        }
    }
    terminal.show_cursor().context("restore console cursor")?;
    Ok(())
}

/// Render a snapshot into plain text using ratatui's test backend.
pub fn render_snapshot_to_text(
    snapshot: &CoreRoomSnapshot,
    width: u16,
    height: u16,
) -> Result<String> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).context("create test console terminal")?;
    terminal
        .draw(|frame| render_console_frame(frame, snapshot))
        .context("draw test console frame")?;
    Ok(buffer_to_string(terminal.backend().buffer()))
}

fn render_console_frame(frame: &mut Frame<'_>, snapshot: &CoreRoomSnapshot) {
    let area = frame.size();
    let layout_model = compute_console_layout(snapshot, area.width);
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(area);

    render_header(frame, root[0], snapshot);
    render_body(frame, root[1], snapshot, layout_model.right_rail.as_ref());
    render_footer(frame, root[2], snapshot);
}

fn render_header(frame: &mut Frame<'_>, area: Rect, snapshot: &CoreRoomSnapshot) {
    let project = &snapshot.project;
    let github = &snapshot.github;
    let title = Line::from(vec![
        Span::styled(
            " CoreRoom ",
            Style::default().fg(Color::Black).bg(Color::Cyan),
        ),
        Span::raw(" Engineering Control Room "),
    ]);
    let lines = vec![
        Line::from(vec![
            Span::styled("Project ", label_style()),
            Span::raw(project.project.clone()),
            Span::raw("  "),
            Span::styled("Repo ", label_style()),
            Span::raw(project.repository.clone()),
            Span::raw("  "),
            Span::styled("Tracker ", label_style()),
            Span::raw(format!("#{}", project.tracker_issue)),
        ]),
        Line::from(vec![
            Span::styled("Branch ", label_style()),
            Span::raw(project.branch.clone()),
            Span::raw("  "),
            Span::styled("Head ", label_style()),
            Span::raw(
                project
                    .head_sha
                    .clone()
                    .unwrap_or_else(|| "unknown".to_owned()),
            ),
            Span::raw("  "),
            dirty_span(project.dirty_state),
        ]),
        Line::from(vec![
            Span::styled("Phase ", label_style()),
            Span::raw(project.active_phase.clone()),
            Span::raw("  "),
            Span::styled("GitHub ", label_style()),
            Span::raw(format!(
                "{} issues / {} PRs / {} failing checks",
                github.open_issues, github.open_pull_requests, github.failing_checks
            )),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_body(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &CoreRoomSnapshot,
    right_rail: Option<&crate::console_layout::RightRailViewModel>,
) {
    let has_rail = right_rail.is_some() && area.width >= 120;
    let chunks = if has_rail {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(70), Constraint::Length(46)])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(100)])
            .split(area)
    };

    render_center(frame, chunks[0], snapshot);
    if let Some(rail) = right_rail.filter(|_| has_rail) {
        render_right_rail(frame, chunks[1], rail);
    }
}

fn render_center(frame: &mut Frame<'_>, area: Rect, snapshot: &CoreRoomSnapshot) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(11), Constraint::Min(8)])
        .split(area);
    let overview = build_console_overview(snapshot);
    render_overview(frame, chunks[0], &overview);
    render_conversation(frame, chunks[1], snapshot);
}

fn render_overview(frame: &mut Frame<'_>, area: Rect, overview: &ConsoleOverview) {
    let header = &overview.header;
    let mut lines = vec![
        Line::from(vec![
            Span::styled("Host ", label_style()),
            Span::raw(format!("@{}  ", header.host_role)),
            Span::styled("Branch ", label_style()),
            Span::raw(header.branch.clone()),
            Span::raw("  "),
            dirty_span(header.dirty_state),
        ]),
        Line::from(vec![
            Span::styled("Phase ", label_style()),
            Span::raw(header.phase.clone()),
            Span::raw("  "),
            Span::styled("Tracker ", label_style()),
            Span::raw(format!("#{}", header.tracker_issue)),
            Span::raw("  "),
            Span::styled("GitHub ", label_style()),
            Span::raw(format!(
                "{} issues / {} PRs / {} failing",
                header.open_issues, header.open_pull_requests, header.failing_checks
            )),
        ]),
        Line::raw(""),
    ];
    for pulse in &overview.pulses {
        lines.push(pulse_line(pulse));
    }
    if let Some(alert) = overview.alerts.first() {
        lines.push(Line::from(vec![
            Span::styled("Alert ", Style::default().fg(Color::Red)),
            Span::raw(alert.title.clone()),
            Span::styled(
                format!(" [{}]", alert.source),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title("Overview"))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn pulse_line(pulse: &OverviewPulse) -> Line<'_> {
    Line::from(vec![
        Span::styled(
            format!("{:<10}", pulse.label),
            Style::default().fg(Color::Cyan),
        ),
        Span::raw(format!("total {:>2}  ", pulse.total)),
        Span::styled(
            format!("ok {:>2}  ", pulse.ok),
            Style::default().fg(Color::Green),
        ),
        Span::styled(
            format!("warn {:>2}  ", pulse.warn),
            Style::default().fg(Color::Yellow),
        ),
        Span::styled(
            format!("block {:>2}  ", pulse.blocking),
            Style::default().fg(Color::Red),
        ),
        Span::styled(
            format!("unknown {:>2}", pulse.unknown),
            Style::default().fg(Color::Gray),
        ),
    ])
}

fn render_conversation(frame: &mut Frame<'_>, area: Rect, snapshot: &CoreRoomSnapshot) {
    let panel = build_public_conversation(snapshot);
    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("Public session: ", label_style()),
        Span::raw("user <-> @"),
        Span::raw(snapshot.runtime.host_role.clone()),
        Span::styled(
            format!(
                "  hidden delegation: {} internal / {} side-rail",
                panel.hidden_internal_count, panel.side_rail_turn_count
            ),
            Style::default().fg(Color::DarkGray),
        ),
    ]));
    lines.push(Line::raw(""));
    for turn in &panel.turns {
        lines.push(Line::from(vec![
            speaker_span(&turn.speaker),
            Span::raw(" "),
        ]));
        lines.extend(wrap_body(&turn.body));
        lines.push(Line::raw(""));
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title("Conversation"))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_right_rail(
    frame: &mut Frame<'_>,
    area: Rect,
    rail: &crate::console_layout::RightRailViewModel,
) {
    let items = rail
        .sections
        .iter()
        .flat_map(section_to_items)
        .collect::<Vec<_>>();
    frame.render_widget(
        List::new(items).block(Block::default().borders(Borders::ALL).title("Control Rail")),
        area,
    );
}

fn section_to_items(section: &RightRailSection) -> Vec<ListItem<'_>> {
    let mut items = vec![ListItem::new(Line::from(vec![Span::styled(
        section.title.clone(),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )]))];
    for row in &section.rows {
        let mut spans = vec![
            Span::styled(
                format!("  {}", row.label),
                Style::default().fg(Color::White),
            ),
            Span::raw(": "),
            status_value_span(&row.value, row.status),
        ];
        if let Some(action) = &row.action {
            spans.push(Span::styled(
                format!(" -> {action}"),
                Style::default().fg(Color::DarkGray),
            ));
        }
        items.push(ListItem::new(Line::from(spans)));
    }
    items.push(ListItem::new(Line::raw("")));
    items
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, snapshot: &CoreRoomSnapshot) {
    let blocking = overview_health_signals(snapshot)
        .iter()
        .filter(|signal| signal.severity == HealthSeverity::Blocking)
        .count();
    let active = snapshot
        .work
        .iter()
        .filter(|work| {
            matches!(
                work.lifecycle,
                WorkLifecycle::InProgress | WorkLifecycle::Ready
            )
        })
        .count();
    let footer = Line::from(vec![
        Span::styled(
            " esc/back/q ",
            Style::default().fg(Color::Black).bg(Color::Cyan),
        ),
        Span::raw(" exit  "),
        Span::styled("snapshot ", label_style()),
        Span::raw(format!("schema {}  ", snapshot.schema_version)),
        Span::styled("active work ", label_style()),
        Span::raw(format!("{active}  ")),
        Span::styled("blocking ", label_style()),
        Span::raw(blocking.to_string()),
    ]);
    frame.render_widget(Paragraph::new(vec![footer]), area);
}

fn status_value_span(value: &str, status: Option<StatusState>) -> Span<'_> {
    Span::styled(
        value.to_owned(),
        status_style(status.unwrap_or(StatusState::Unknown)),
    )
}

fn status_style(status: StatusState) -> Style {
    match status {
        StatusState::Ok => Style::default().fg(Color::Green),
        StatusState::Warn => Style::default().fg(Color::Yellow),
        StatusState::Blocking => Style::default().fg(Color::Red),
        StatusState::Unknown => Style::default().fg(Color::Gray),
    }
}

fn dirty_span(state: DirtyState) -> Span<'static> {
    match state {
        DirtyState::Clean => Span::styled("clean", Style::default().fg(Color::Green)),
        DirtyState::Dirty => Span::styled("dirty", Style::default().fg(Color::Yellow)),
        DirtyState::Unknown => Span::styled("dirty unknown", Style::default().fg(Color::Gray)),
    }
}

fn speaker_span(speaker: &str) -> Span<'_> {
    let style = if speaker == "user" {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else if speaker == "host" {
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };
    Span::styled(format!("@{speaker}"), style)
}

fn wrap_body(body: &str) -> Vec<Line<'_>> {
    body.lines()
        .map(|line| Line::from(vec![Span::raw(format!("  {line}"))]))
        .collect()
}

fn label_style() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}

fn is_exit_key(code: KeyCode) -> bool {
    matches!(
        code,
        KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('q' | 'b')
    )
}

fn buffer_to_string(buffer: &Buffer) -> String {
    let mut lines = Vec::new();
    for y in 0..buffer.area.height {
        let mut line = String::new();
        for x in 0..buffer.area.width {
            line.push_str(buffer.get(x, y).symbol());
        }
        lines.push(line.trim_end().to_owned());
    }
    lines.join("\n")
}

fn write_enter_commands<W: Write>(mut writer: W) -> io::Result<()> {
    execute!(writer, EnterAlternateScreen, Hide)
}

fn write_leave_commands<W: Write>(mut writer: W) -> io::Result<()> {
    execute!(writer, Show, LeaveAlternateScreen)
}

#[derive(Debug)]
struct ConsoleTerminalGuard;

impl ConsoleTerminalGuard {
    fn enter() -> Result<Self> {
        terminal::enable_raw_mode().context("enable console raw mode")?;
        if let Err(error) = write_enter_commands(io::stdout()) {
            let _ = terminal::disable_raw_mode();
            return Err(error).context("enter console alternate screen");
        }
        Ok(Self)
    }
}

impl Drop for ConsoleTerminalGuard {
    fn drop(&mut self) {
        let _ = write_leave_commands(io::stdout());
        let _ = terminal::disable_raw_mode();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_enter_and_leave_commands_are_balanced() {
        let mut enter = Vec::new();
        write_enter_commands(&mut enter).expect("enter commands");
        let mut leave = Vec::new();
        write_leave_commands(&mut leave).expect("leave commands");

        let enter = String::from_utf8_lossy(&enter);
        let leave = String::from_utf8_lossy(&leave);
        assert!(enter.contains("[?1049h"), "enter alternate screen");
        assert!(enter.contains("[?25l"), "hide cursor");
        assert!(leave.contains("[?25h"), "show cursor");
        assert!(leave.contains("[?1049l"), "leave alternate screen");
    }

    #[test]
    fn exit_key_set_matches_console_contract() {
        assert!(is_exit_key(KeyCode::Esc));
        assert!(is_exit_key(KeyCode::Backspace));
        assert!(is_exit_key(KeyCode::Char('q')));
        assert!(is_exit_key(KeyCode::Char('b')));
        assert!(!is_exit_key(KeyCode::Char('x')));
    }
}
