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
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use crate::console_actions::ConsolePermissionOverlay;
use crate::console_conversation::build_public_conversation;
use crate::console_health::overview_health_signals;
use crate::console_layout::{compute_console_layout, RightRailSection};
use crate::console_navigation::{visible_rows, ConsoleNavigator, ConsoleView, ConsoleVisibleRow};
use crate::console_overview::{build_console_overview, ConsoleOverview, OverviewPulse};
use crate::console_snapshot::{
    CoreRoomSnapshot, DirtyState, HealthSeverity, InternalDelegationActivity,
    InternalDelegationState, StatusState, WorkLifecycle,
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

/// Run the interactive read-only full-screen console for a live local project.
pub fn run_live_console(project_root: &Path) -> Result<()> {
    let snapshot = crate::console_live::snapshot_from_project(project_root)?;
    run_console(&snapshot)
}

/// Run the interactive read-only full-screen console for a snapshot file.
pub fn run_snapshot_console(snapshot_path: &Path) -> Result<()> {
    let snapshot = load_snapshot(snapshot_path)?;
    run_console(&snapshot)
}

/// Run the interactive read-only full-screen console for a validated snapshot.
pub fn run_console(snapshot: &CoreRoomSnapshot) -> Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        anyhow::bail!("cr console requires an interactive TTY");
    }

    let _guard = ConsoleTerminalGuard::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend).context("create console terminal")?;
    let mut navigator = ConsoleNavigator::default();
    terminal.clear().context("clear console terminal")?;

    loop {
        terminal
            .draw(|frame| render_console_frame_with_nav(frame, snapshot, &navigator))
            .context("render console frame")?;
        if event::poll(Duration::from_millis(200)).context("poll console input")? {
            match event::read().context("read console input")? {
                Event::Key(key) if key.kind == KeyEventKind::Press && is_exit_key(key.code) => {
                    break;
                }
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    let row_count = visible_rows(snapshot, &[], &navigator).len();
                    navigator.apply_key(key.code, key.modifiers, row_count);
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
    render_snapshot_to_text_with_nav(snapshot, width, height, &ConsoleNavigator::default())
}

/// Render a snapshot with an explicit navigation state into plain text.
pub fn render_snapshot_to_text_with_nav(
    snapshot: &CoreRoomSnapshot,
    width: u16,
    height: u16,
    navigator: &ConsoleNavigator,
) -> Result<String> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).context("create test console terminal")?;
    terminal
        .draw(|frame| render_console_frame_with_nav(frame, snapshot, navigator))
        .context("draw test console frame")?;
    Ok(buffer_to_string(terminal.backend().buffer()))
}

/// Render a snapshot with an explicit action overlay into plain text.
pub fn render_snapshot_to_text_with_action_overlay(
    snapshot: &CoreRoomSnapshot,
    width: u16,
    height: u16,
    navigator: &ConsoleNavigator,
    overlay: &ConsolePermissionOverlay,
) -> Result<String> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).context("create test console terminal")?;
    terminal
        .draw(|frame| {
            render_console_frame_with_nav(frame, snapshot, navigator);
            render_action_overlay(frame, frame.size(), overlay);
        })
        .context("draw test console frame")?;
    Ok(buffer_to_string(terminal.backend().buffer()))
}

fn render_console_frame_with_nav(
    frame: &mut Frame<'_>,
    snapshot: &CoreRoomSnapshot,
    navigator: &ConsoleNavigator,
) {
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
    render_body(
        frame,
        root[1],
        snapshot,
        navigator,
        layout_model.right_rail.as_ref(),
    );
    render_footer(frame, root[2], snapshot, navigator);
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
                    .unwrap_or_else(|| "not observed".to_owned()),
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
    navigator: &ConsoleNavigator,
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

    render_center(frame, chunks[0], snapshot, navigator);
    if let Some(rail) = right_rail.filter(|_| has_rail) {
        render_right_rail(frame, chunks[1], rail);
    }
}

fn render_center(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &CoreRoomSnapshot,
    navigator: &ConsoleNavigator,
) {
    if navigator.active_view != ConsoleView::Overview {
        render_active_view(frame, area, snapshot, navigator);
        return;
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(11), Constraint::Min(8)])
        .split(area);
    let overview = build_console_overview(snapshot);
    render_overview(frame, chunks[0], &overview);
    render_conversation(frame, chunks[1], snapshot);
}

fn render_active_view(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &CoreRoomSnapshot,
    navigator: &ConsoleNavigator,
) {
    let rows = visible_rows(snapshot, &[], navigator);
    let items = if rows.is_empty() {
        vec![ListItem::new(Line::from(vec![Span::styled(
            "No rows for this view",
            Style::default().fg(Color::DarkGray),
        )]))]
    } else {
        rows.iter()
            .enumerate()
            .map(|(index, row)| active_view_item(index, row, navigator))
            .collect::<Vec<_>>()
    };
    let title = format!(
        "{}{}",
        navigator.active_view.label(),
        if navigator.detail_open { " detail" } else { "" }
    );
    frame.render_widget(
        List::new(items).block(Block::default().borders(Borders::ALL).title(title)),
        area,
    );
}

fn render_action_overlay(frame: &mut Frame<'_>, area: Rect, overlay: &ConsolePermissionOverlay) {
    let width = area.width.saturating_sub(8).clamp(40, 92);
    let height = area.height.saturating_sub(6).clamp(8, 13);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    let rect = Rect {
        x,
        y,
        width,
        height,
    };
    let mut lines = vec![
        Line::from(vec![Span::styled(
            overlay.title.clone(),
            status_style(overlay.status).add_modifier(Modifier::BOLD),
        )]),
        Line::raw(""),
    ];
    lines.extend(overlay.lines.iter().map(|line| Line::raw(line.clone())));
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title("Host Action"))
            .wrap(Wrap { trim: true }),
        rect,
    );
}

fn active_view_item<'a>(
    index: usize,
    row: &'a ConsoleVisibleRow,
    navigator: &ConsoleNavigator,
) -> ListItem<'a> {
    let marker = if index == navigator.selected {
        ">"
    } else {
        " "
    };
    let mut spans = vec![
        Span::styled(marker, Style::default().fg(Color::Cyan)),
        Span::raw(" "),
        Span::styled(row.primary.clone(), status_style(row.status)),
        Span::raw("  "),
        Span::raw(row.secondary.clone()),
    ];
    if navigator.detail_open && index == navigator.selected {
        if let Some(source) = &row.source {
            spans.push(Span::styled(
                format!("  [{source}]"),
                Style::default().fg(Color::DarkGray),
            ));
        }
    }
    ListItem::new(Line::from(spans))
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
    let mut spans = vec![
        Span::styled(
            format!("{:<10}", pulse.label),
            Style::default().fg(Color::Cyan),
        ),
        Span::raw(format!("total {:>2}  ", pulse.total)),
    ];
    if pulse.ok > 0 {
        spans.push(Span::styled(
            format!("ok {:>2}  ", pulse.ok),
            Style::default().fg(Color::Green),
        ));
    }
    if pulse.warn > 0 {
        spans.push(Span::styled(
            format!("warn {:>2}  ", pulse.warn),
            Style::default().fg(Color::Yellow),
        ));
    }
    if pulse.blocking > 0 {
        spans.push(Span::styled(
            format!("block {:>2}  ", pulse.blocking),
            Style::default().fg(Color::Red),
        ));
    }
    if pulse.unknown > 0 {
        spans.push(Span::styled(
            format!("not observed {:>2}", pulse.unknown),
            Style::default().fg(Color::Gray),
        ));
    }
    Line::from(spans)
}

fn render_conversation(frame: &mut Frame<'_>, area: Rect, snapshot: &CoreRoomSnapshot) {
    let panel = build_public_conversation(snapshot);
    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("Public conversation: ", label_style()),
        Span::raw("@user <-> @"),
        Span::raw(snapshot.runtime.host_role.clone()),
    ]));
    if panel.hidden_internal_count > 0 || !panel.internal_activity.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Internal work: ", label_style()),
            Span::raw(format!(
                "{} hidden turns · {} task cards",
                panel.hidden_internal_count,
                panel.internal_activity.len()
            )),
            Span::styled(
                "  surfaced only when user @mentions a role, or @host summarizes risk/evidence",
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }
    lines.push(Line::raw(""));
    if panel.turns.is_empty() {
        lines.push(Line::from(vec![
            speaker_span("user", &snapshot.runtime.host_role),
            Span::raw(" "),
        ]));
        lines.push(Line::from(vec![Span::styled(
            "  No public request in this room yet.",
            Style::default().fg(Color::DarkGray),
        )]));
        lines.push(Line::from(vec![
            speaker_span(&snapshot.runtime.host_role, &snapshot.runtime.host_role),
            Span::raw(" "),
        ]));
        lines.push(Line::from(vec![Span::raw(
            "  Press q to enter the REPL and type your request. This pane is reserved for user-facing input/output.",
        )]));
        lines.push(Line::raw(""));
    } else {
        for turn in &panel.turns {
            lines.push(Line::from(vec![
                speaker_span(&turn.speaker, &snapshot.runtime.host_role),
                if is_public_specialist(&turn.speaker, &snapshot.runtime.host_role) {
                    Span::styled(" direct", Style::default().fg(Color::DarkGray))
                } else {
                    Span::raw("")
                },
            ]));
            lines.extend(wrap_body(&turn.body));
            lines.push(Line::raw(""));
        }
    }
    if !panel.internal_activity.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "Host-managed task cards",
            label_style(),
        )]));
        for activity in panel.internal_activity.iter().take(3) {
            lines.extend(delegation_card_lines(activity));
        }
        let remaining = panel.internal_activity.len().saturating_sub(3);
        if remaining > 0 {
            lines.push(Line::from(vec![Span::styled(
                format!("  +{remaining} more in Xray/log views"),
                Style::default().fg(Color::DarkGray),
            )]));
        }
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

fn render_footer(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &CoreRoomSnapshot,
    navigator: &ConsoleNavigator,
) {
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
            " tab/backtab ",
            Style::default().fg(Color::Black).bg(Color::Cyan),
        ),
        Span::raw(" switch  "),
        Span::styled("enter/esc ", label_style()),
        Span::raw("detail  "),
        Span::styled("q ", label_style()),
        Span::raw("exit  "),
        Span::styled("view ", label_style()),
        Span::raw(format!("{}  ", navigator.active_view.label())),
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
        DirtyState::Unknown => {
            Span::styled("changes not observed", Style::default().fg(Color::Gray))
        }
    }
}

fn speaker_span<'a>(speaker: &'a str, host_role: &str) -> Span<'a> {
    let style = if speaker == "user" {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else if speaker == host_role || speaker == "host" {
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };
    Span::styled(format!("@{speaker}"), style)
}

fn is_public_specialist(speaker: &str, host_role: &str) -> bool {
    speaker != "user" && speaker != host_role && speaker != "host"
}

fn delegation_card_lines(activity: &InternalDelegationActivity) -> Vec<Line<'_>> {
    let state_style = match activity.state {
        InternalDelegationState::Blocked => Style::default().fg(Color::Red),
        InternalDelegationState::Completed => Style::default().fg(Color::Green),
        InternalDelegationState::Dispatched
        | InternalDelegationState::Working
        | InternalDelegationState::Reviewing => Style::default().fg(Color::Yellow),
    };
    let mut header = vec![
        Span::styled("  [", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("@{}", activity.role),
            Style::default().fg(Color::Cyan),
        ),
        Span::styled("] ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{:?}", activity.state).to_lowercase(),
            state_style.add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some(work_order) = &activity.work_order {
        header.push(Span::styled(
            format!(" · {work_order}"),
            Style::default().fg(Color::DarkGray),
        ));
    }
    let mut lines = vec![
        Line::raw(""),
        Line::from(header),
        Line::from(vec![Span::raw(format!("    {}", activity.summary))]),
    ];
    if let Some(xray_ref) = &activity.xray_ref {
        lines.push(Line::from(vec![Span::styled(
            format!("    detail: {xray_ref}"),
            Style::default().fg(Color::DarkGray),
        )]));
    }
    lines
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
