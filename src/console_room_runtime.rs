//! Full-screen ratatui host for the executable CoreRoom runtime.
//!
//! This surface is not a snapshot viewer. It feeds submitted composer
//! lines into the existing REPL parser and renders the same `RoomEvent`
//! stream that `cr start` writes to stdout.

use std::collections::BTreeMap;
use std::io::{self, IsTerminal as _, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::cursor::{Hide, Show};
use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::{CrosstermBackend, TestBackend};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use tokio::sync::mpsc;
use unicode_width::UnicodeWidthChar;

use crate::config::Config;
use crate::console_composer::{
    ComposerState, ComposerSubmissionState, ComposerSubmitOutcome, ComposerViewModel,
};
use crate::crep::CrepEvent;
use crate::output::work_card::{WorkCard, WorkStatus};
use crate::permissions::{BridgeRequest, BridgeResponse, DecisionScope, PermissionDecision};
use crate::repl::{Command, RuntimeInput};
use crate::room_io::{NoticeLevel, RoomEvent, RoomSink, SpinnerPaint, SpinnerSnapshot, StdoutSink};
use crate::room_io_tui::TuiSink;

/// Mutable render state for the executable room.
#[derive(Debug)]
pub struct RoomRuntimeState {
    project_root: PathBuf,
    project_name: String,
    host_role: String,
    composer: ComposerState,
    scrollback: Vec<String>,
    spinners: BTreeMap<String, SpinnerSnapshot>,
    work_cards: BTreeMap<String, WorkCard>,
    permission: Option<PendingPermission>,
    exiting: bool,
}

#[derive(Debug, Clone)]
struct PendingPermission {
    request: BridgeRequest,
    host_role: String,
    response_tx: Option<mpsc::UnboundedSender<BridgeResponse>>,
}

impl RoomRuntimeState {
    /// Build initial state from project config, falling back to a host-only
    /// composer while the REPL auto-initializes a new `.coreroom/`.
    #[must_use]
    pub fn for_project(project_root: &Path) -> Self {
        let cfg = Config::load(project_root).ok();
        let host_role = cfg
            .as_ref()
            .map_or_else(|| "host".to_owned(), |cfg| cfg.host_role.clone());
        let mut roles = cfg.as_ref().map_or_else(
            || vec![host_role.clone()],
            |cfg| cfg.role_names().map(ToOwned::to_owned).collect::<Vec<_>>(),
        );
        if roles.is_empty() {
            roles.push(host_role.clone());
        }
        Self::new(project_root.to_path_buf(), host_role, roles)
    }

    fn new(project_root: PathBuf, host_role: String, roles: Vec<String>) -> Self {
        let project_name = project_root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("CoreRoom")
            .to_owned();
        let composer = ComposerState::new(
            roles,
            crate::repl::composer_command_specs(),
            "type a task · @role · /help · /exit",
        );
        Self {
            project_root,
            project_name,
            host_role,
            composer,
            scrollback: Vec::new(),
            spinners: BTreeMap::new(),
            work_cards: BTreeMap::new(),
            permission: None,
            exiting: false,
        }
    }

    /// Apply one runtime event to the render model.
    pub fn apply_event(&mut self, event: RoomEvent) {
        match event {
            RoomEvent::Crep { event, host_role } => {
                let ends_turn = matches!(
                    event.as_ref(),
                    CrepEvent::RoleSpoke { .. }
                        | CrepEvent::RoleStopped { .. }
                        | CrepEvent::TurnInterrupted { .. }
                );
                let rendered = StdoutSink::render_to_string(&RoomEvent::Crep { event, host_role });
                self.push_rendered(&rendered);
                if ends_turn && !self.has_active_work() {
                    self.composer
                        .set_submission_state(ComposerSubmissionState::Idle);
                }
            }
            RoomEvent::Notice { level, text } => self.push_notice(level, text),
            RoomEvent::Banner(text) => self.push_rendered(&text),
            RoomEvent::WorkCard(card) => {
                self.work_cards.insert(card.role.clone(), card);
                if !self.has_active_work() {
                    self.composer
                        .set_submission_state(ComposerSubmissionState::Idle);
                }
            }
            RoomEvent::Spinner(snapshot) => {
                match snapshot.paint {
                    SpinnerPaint::Cleared => {
                        self.spinners.remove(&snapshot.role);
                    }
                    SpinnerPaint::Painting | SpinnerPaint::WaitingApproval => {
                        self.spinners.insert(snapshot.role.clone(), snapshot);
                    }
                }
                self.composer
                    .set_submission_state(if self.permission.is_some() {
                        ComposerSubmissionState::Blocked
                    } else if self.has_active_work() {
                        ComposerSubmissionState::Submitting
                    } else {
                        ComposerSubmissionState::Idle
                    });
            }
            RoomEvent::PermissionPrompt {
                request,
                host_role,
                response_tx,
            } => {
                self.push_notice(
                    NoticeLevel::Warn,
                    format!("@{} wants {} approval", request.role, request.tool),
                );
                self.permission = Some(PendingPermission {
                    request,
                    host_role,
                    response_tx,
                });
                self.composer
                    .set_submission_state(ComposerSubmissionState::Blocked);
            }
            RoomEvent::PermissionOutcome {
                role,
                host_role,
                response,
            } => {
                if self
                    .permission
                    .as_ref()
                    .is_some_and(|pending| pending.request.role == role)
                {
                    self.permission = None;
                }
                let rendered = StdoutSink::render_to_string(&RoomEvent::PermissionOutcome {
                    role,
                    host_role,
                    response,
                });
                self.push_rendered(&rendered);
                self.composer
                    .set_submission_state(if self.has_active_work() {
                        ComposerSubmissionState::Submitting
                    } else {
                        ComposerSubmissionState::Idle
                    });
            }
        }
    }

    fn push_user_line(&mut self, line: &str) {
        self.push_scrollback(format!("@user {line}"));
    }

    fn push_notice(&mut self, level: NoticeLevel, text: impl Into<String>) {
        let label = match level {
            NoticeLevel::Ok => "ok",
            NoticeLevel::Warn => "warn",
            NoticeLevel::Bad => "error",
            NoticeLevel::Hint => "hint",
            NoticeLevel::System => "system",
        };
        self.push_scrollback(format!("{label}: {}", text.into()));
    }

    fn push_rendered(&mut self, text: &str) {
        for line in strip_ansi(text).lines() {
            self.push_scrollback(line.trim_end().to_owned());
        }
    }

    fn push_scrollback(&mut self, line: String) {
        self.scrollback.push(line);
        let overflow = self.scrollback.len().saturating_sub(1000);
        if overflow > 0 {
            self.scrollback.drain(0..overflow);
        }
    }

    fn has_active_work(&self) -> bool {
        self.spinners
            .values()
            .any(|snapshot| snapshot.paint != SpinnerPaint::Cleared)
            || self
                .work_cards
                .values()
                .any(|card| matches!(card.status, WorkStatus::Working { .. }))
    }

    fn active_work_count(&self) -> usize {
        self.spinners.len()
            + self
                .work_cards
                .values()
                .filter(|card| matches!(card.status, WorkStatus::Working { .. }))
                .count()
    }
}

/// Run the executable full-screen room against a local CoreRoom project.
pub async fn run_live_room(project_root: &Path, options: crate::repl::RunOptions) -> Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        anyhow::bail!("cr console --live-room requires an interactive TTY");
    }

    let mut state = RoomRuntimeState::for_project(project_root);
    let (tui_sink, mut event_rx) = TuiSink::channel();
    let sink: Arc<dyn RoomSink> = Arc::new(tui_sink);
    let (input_tx, input_rx) = mpsc::unbounded_channel();
    let runtime_root = project_root.to_path_buf();
    let runtime_task = tokio::spawn(async move {
        crate::repl::run_with_options_and_sink_input(&runtime_root, options, sink, input_rx).await
    });

    let _guard = RoomTerminalGuard::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend).context("create live room terminal")?;
    terminal.clear().context("clear live room terminal")?;

    let result = drive_live_room(
        &mut terminal,
        &mut state,
        &mut event_rx,
        &input_tx,
        runtime_task,
    )
    .await;
    let _ = terminal.show_cursor();
    result
}

async fn drive_live_room(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut RoomRuntimeState,
    event_rx: &mut mpsc::UnboundedReceiver<RoomEvent>,
    input_tx: &mpsc::UnboundedSender<RuntimeInput>,
    runtime_task: tokio::task::JoinHandle<Result<()>>,
) -> Result<()> {
    loop {
        while let Ok(event) = event_rx.try_recv() {
            state.apply_event(event);
        }
        if let Err(error) = terminal.draw(|frame| render_room_runtime_frame(frame, state)) {
            let _ = input_tx.send(RuntimeInput::Eof);
            runtime_task.abort();
            return Err(error).context("render live room frame");
        }

        if runtime_task.is_finished() {
            break;
        }
        let has_input = match event::poll(Duration::from_millis(50)) {
            Ok(has_input) => has_input,
            Err(error) => {
                let _ = input_tx.send(RuntimeInput::Eof);
                runtime_task.abort();
                return Err(error).context("poll live room input");
            }
        };
        if has_input {
            let event = match event::read() {
                Ok(event) => event,
                Err(error) => {
                    let _ = input_tx.send(RuntimeInput::Eof);
                    runtime_task.abort();
                    return Err(error).context("read live room input");
                }
            };
            handle_terminal_event(event, state, input_tx)?;
        }
    }
    let runtime_result = runtime_task.await.context("joining live room runtime")?;
    runtime_result
}

fn handle_terminal_event(
    event: Event,
    state: &mut RoomRuntimeState,
    input_tx: &mpsc::UnboundedSender<RuntimeInput>,
) -> Result<()> {
    if state.exiting {
        return Ok(());
    }
    match event {
        Event::Paste(text) if state.permission.is_none() => {
            state.composer.paste_str(&text);
        }
        Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
            handle_key(key, state, input_tx)?;
        }
        _ => {}
    }
    Ok(())
}

fn handle_key(
    key: KeyEvent,
    state: &mut RoomRuntimeState,
    input_tx: &mpsc::UnboundedSender<RuntimeInput>,
) -> Result<()> {
    if state.permission.is_some() {
        if let Some(response) = permission_response_for_key(key) {
            answer_permission(state, response);
        }
        return Ok(());
    }

    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if state.has_active_work() {
                raise_runtime_interrupt()?;
                state.push_notice(NoticeLevel::System, "interrupt requested");
            } else {
                let _ = input_tx.send(RuntimeInput::Interrupted);
            }
        }
        KeyCode::Char('d')
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && state.composer.view_model().input.is_empty() =>
        {
            state.exiting = true;
            let _ = input_tx.send(RuntimeInput::Eof);
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.composer.clear();
        }
        KeyCode::Tab | KeyCode::Down if state.composer.cycle_completion() => {}
        KeyCode::Up if state.composer.cycle_completion_back() => {}
        KeyCode::Right | KeyCode::Char('f')
            if matches!(key.code, KeyCode::Right)
                || key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            if state.composer.view_model().ghost_suffix.is_some() {
                let _ = state.composer.accept_completion();
            } else {
                let _ = state.composer.move_right();
            }
        }
        KeyCode::Esc => {
            let _ = state.composer.dismiss_completion();
        }
        KeyCode::Backspace => {
            let _ = state.composer.backspace();
        }
        KeyCode::Delete => {
            let _ = state.composer.delete();
        }
        KeyCode::Left => {
            let _ = state.composer.move_left();
        }
        KeyCode::Home => {
            let _ = state.composer.move_home();
        }
        KeyCode::End => {
            let _ = state.composer.move_end();
        }
        KeyCode::Enter
            if key
                .modifiers
                .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) =>
        {
            state.composer.insert_newline();
        }
        KeyCode::Enter => match state.composer.submit() {
            ComposerSubmitOutcome::Submitted(line) => submit_line(state, input_tx, line)?,
            ComposerSubmitOutcome::CompletionAccepted | ComposerSubmitOutcome::Noop => {}
        },
        KeyCode::Char(ch)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            state.composer.insert_char(ch);
        }
        _ => {}
    }
    Ok(())
}

fn submit_line(
    state: &mut RoomRuntimeState,
    input_tx: &mpsc::UnboundedSender<RuntimeInput>,
    line: String,
) -> Result<()> {
    state.push_user_line(&line);
    match crate::repl::parse_line(&line) {
        Command::Halt(_) if state.has_active_work() => {
            raise_runtime_interrupt()?;
            state.push_notice(NoticeLevel::System, "interrupt requested");
        }
        Command::Exit => {
            state.exiting = true;
            let _ = input_tx.send(RuntimeInput::Line(line));
        }
        _ => {
            input_tx
                .send(RuntimeInput::Line(line))
                .context("sending composer line to room runtime")?;
        }
    }
    Ok(())
}

fn answer_permission(state: &mut RoomRuntimeState, response: BridgeResponse) {
    if let Some(pending) = state.permission.as_ref() {
        if let Some(tx) = &pending.response_tx {
            let _ = tx.send(response);
        }
        state
            .composer
            .set_submission_state(ComposerSubmissionState::Submitting);
    }
}

fn permission_response_for_key(key: KeyEvent) -> Option<BridgeResponse> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
        return Some(BridgeResponse::deny("declined: cancelled at prompt"));
    }
    match key.code {
        KeyCode::Char('a' | 'A' | 'y' | 'Y') => Some(permission_decision(
            PermissionDecision::Allow,
            DecisionScope::Once,
        )),
        KeyCode::Char('s' | 'S') => Some(permission_decision(
            PermissionDecision::Allow,
            DecisionScope::Session,
        )),
        KeyCode::Char('d' | 'D') => Some(permission_decision(
            PermissionDecision::Deny,
            DecisionScope::Once,
        )),
        KeyCode::Char('n' | 'N') | KeyCode::Esc => Some(permission_decision(
            PermissionDecision::Deny,
            DecisionScope::Session,
        )),
        _ => None,
    }
}

fn permission_decision(decision: PermissionDecision, scope: DecisionScope) -> BridgeResponse {
    let reason = match (decision, scope) {
        (PermissionDecision::Allow, DecisionScope::Once) => "user allowed (once)",
        (PermissionDecision::Allow, DecisionScope::Session) => "user allowed (session)",
        (PermissionDecision::Deny, DecisionScope::Once) => "user denied (once)",
        (PermissionDecision::Deny, DecisionScope::Session) => "user denied (session)",
    };
    BridgeResponse {
        v: 1,
        decision,
        scope,
        reason: reason.to_owned(),
    }
}

fn raise_runtime_interrupt() -> Result<()> {
    signal_hook::low_level::raise(signal_hook::consts::SIGINT)
        .context("raising runtime interrupt signal")
}

/// Render a runtime state into text using ratatui's test backend.
pub fn render_room_runtime_to_text(
    state: &RoomRuntimeState,
    width: u16,
    height: u16,
) -> Result<String> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).context("create test live room terminal")?;
    terminal
        .draw(|frame| render_room_runtime_frame(frame, state))
        .context("draw test live room frame")?;
    Ok(buffer_to_string(terminal.backend().buffer()))
}

fn render_room_runtime_frame(frame: &mut Frame<'_>, state: &RoomRuntimeState) {
    let area = frame.area();
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(5),
            Constraint::Length(1),
        ])
        .split(area);

    render_header(frame, root[0], state);
    render_body(frame, root[1], state);
    render_composer(frame, root[2], state);
    render_footer(frame, root[3], state);
    if let Some(permission) = &state.permission {
        render_permission_overlay(frame, area, permission);
    }
}

fn render_header(frame: &mut Frame<'_>, area: Rect, state: &RoomRuntimeState) {
    let status = if state.exiting {
        "exiting"
    } else if state.permission.is_some() {
        "waiting approval"
    } else if state.has_active_work() {
        "working"
    } else {
        "idle"
    };
    let line = Line::from(vec![
        Span::styled("Project ", label_style()),
        Span::raw(state.project_name.clone()),
        Span::raw("  "),
        Span::styled("Host ", label_style()),
        Span::raw(format!("@{}  ", state.host_role)),
        Span::styled("Status ", label_style()),
        Span::raw(status),
        Span::raw("  "),
        Span::styled("Work ", label_style()),
        Span::raw(state.active_work_count().to_string()),
    ]);
    frame.render_widget(
        Paragraph::new(vec![line])
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("CoreRoom Runtime"),
            )
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_body(frame: &mut Frame<'_>, area: Rect, state: &RoomRuntimeState) {
    if area.width >= 112 {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(62), Constraint::Length(42)])
            .split(area);
        render_scrollback(frame, chunks[0], state);
        render_status_rail(frame, chunks[1], state);
    } else {
        render_scrollback(frame, area, state);
    }
}

fn render_scrollback(frame: &mut Frame<'_>, area: Rect, state: &RoomRuntimeState) {
    let visible_rows = area.height.saturating_sub(2) as usize;
    let start = state.scrollback.len().saturating_sub(visible_rows);
    let items = if state.scrollback.is_empty() {
        vec![ListItem::new(Line::from(vec![Span::styled(
            "Submit a task below. Runtime output appears here.",
            Style::default().fg(Color::DarkGray),
        )]))]
    } else {
        state.scrollback[start..]
            .iter()
            .map(|line| ListItem::new(Line::raw(line.clone())))
            .collect()
    };
    frame.render_widget(
        List::new(items).block(Block::default().borders(Borders::ALL).title("Room")),
        area,
    );
}

fn render_status_rail(frame: &mut Frame<'_>, area: Rect, state: &RoomRuntimeState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(8), Constraint::Min(6)])
        .split(area);
    render_role_lane(frame, chunks[0], state);
    render_work_cards(frame, chunks[1], state);
}

fn render_role_lane(frame: &mut Frame<'_>, area: Rect, state: &RoomRuntimeState) {
    let mut lines = Vec::new();
    if state.spinners.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "idle",
            Style::default().fg(Color::DarkGray),
        )]));
    } else {
        for snapshot in state.spinners.values() {
            lines.push(spinner_line(snapshot));
        }
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title("Roles"))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn spinner_line(snapshot: &SpinnerSnapshot) -> Line<'static> {
    const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let frame = FRAMES[snapshot.frame % FRAMES.len()];
    let state = snapshot
        .current_state
        .clone()
        .unwrap_or_else(|| "thinking".to_owned());
    let style = match snapshot.paint {
        SpinnerPaint::WaitingApproval => Style::default().fg(Color::Yellow),
        SpinnerPaint::Painting => Style::default().fg(Color::Cyan),
        SpinnerPaint::Cleared => Style::default().fg(Color::DarkGray),
    };
    Line::from(vec![
        Span::styled(frame, style),
        Span::raw(" "),
        Span::styled(
            format!("@{}", snapshot.role),
            style.add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            " · {}s · ",
            snapshot.started_at.elapsed().as_secs()
        )),
        Span::raw(state),
    ])
}

fn render_work_cards(frame: &mut Frame<'_>, area: Rect, state: &RoomRuntimeState) {
    let mut lines = Vec::new();
    if state.work_cards.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "no work cards yet",
            Style::default().fg(Color::DarkGray),
        )]));
    } else {
        let width = usize::from(area.width.saturating_sub(4)).max(28);
        for card in state.work_cards.values().rev().take(3) {
            for line in strip_ansi(&card.render(width)).lines() {
                lines.push(Line::raw(line.to_owned()));
            }
            lines.push(Line::raw(""));
        }
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title("Work"))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_composer(frame: &mut Frame<'_>, area: Rect, state: &RoomRuntimeState) {
    let vm = state.composer.view_model();
    let title = match vm.submission_state {
        ComposerSubmissionState::Idle => format!("Ask @{}", state.host_role),
        ComposerSubmissionState::Submitting => "Runtime active".to_owned(),
        ComposerSubmissionState::Blocked => "Permission required".to_owned(),
    };
    frame.render_widget(
        Paragraph::new(composer_lines(&vm))
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: false }),
        area,
    );
    if vm.submission_state != ComposerSubmissionState::Blocked {
        frame.set_cursor_position(composer_cursor_position(area, &vm));
    }
}

fn composer_lines(vm: &ComposerViewModel) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let input = vm.input.clone();
    if input.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("cr > ", Style::default().fg(Color::Green)),
            Span::styled(vm.prompt_hint.clone(), Style::default().fg(Color::DarkGray)),
        ]));
    } else {
        for (index, line) in input.lines().enumerate() {
            let mut spans = Vec::new();
            if index == 0 {
                spans.push(Span::styled("cr > ", Style::default().fg(Color::Green)));
            } else {
                spans.push(Span::raw("     "));
            }
            spans.push(Span::raw(line.to_owned()));
            if index == input.lines().count().saturating_sub(1) {
                if let Some(ghost) = &vm.ghost_suffix {
                    spans.push(Span::styled(
                        ghost.clone(),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
            }
            lines.push(Line::from(spans));
        }
    }
    if !vm.candidates.is_empty() {
        let menu = vm
            .candidates
            .iter()
            .take(4)
            .map(|candidate| {
                if candidate.selected {
                    format!("[{}]", candidate.label)
                } else {
                    candidate.label.clone()
                }
            })
            .collect::<Vec<_>>()
            .join("  ");
        lines.push(Line::from(vec![Span::styled(
            menu,
            Style::default().fg(Color::DarkGray),
        )]));
    }
    lines
}

fn composer_cursor_position(area: Rect, vm: &ComposerViewModel) -> (u16, u16) {
    let inner_x = area.x.saturating_add(1);
    let inner_y = area.y.saturating_add(1);
    let inner_width = area.width.saturating_sub(2);
    let inner_height = area.height.saturating_sub(2).max(1);
    let (row, col) = cursor_row_col(&vm.input, vm.cursor);
    let prompt_width = 5;
    let max_col = usize::from(inner_width.saturating_sub(1));
    let cursor_col = u16::try_from(col.min(max_col)).unwrap_or(u16::MAX);
    let cursor_row = u16::try_from(row).unwrap_or(u16::MAX);
    let x = inner_x
        .saturating_add(prompt_width)
        .saturating_add(cursor_col)
        .min(inner_x.saturating_add(inner_width.saturating_sub(1)));
    let y = inner_y
        .saturating_add(cursor_row.min(inner_height.saturating_sub(1)))
        .min(inner_y.saturating_add(inner_height.saturating_sub(1)));
    (x, y)
}

fn cursor_row_col(input: &str, cursor: usize) -> (usize, usize) {
    let mut row = 0usize;
    let mut col = 0usize;
    for ch in input.chars().take(cursor) {
        if ch == '\n' {
            row += 1;
            col = 0;
        } else {
            col += ch.width().unwrap_or(0);
        }
    }
    (row, col)
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, state: &RoomRuntimeState) {
    let footer = Line::from(vec![
        Span::styled(" enter ", Style::default().fg(Color::Black).bg(Color::Cyan)),
        Span::raw("send  "),
        Span::styled(" shift/alt+enter ", label_style()),
        Span::raw("newline  "),
        Span::styled(" ctrl-c ", label_style()),
        Span::raw("halt  "),
        Span::styled(" ctrl-d ", label_style()),
        Span::raw("exit  "),
        Span::styled(" path ", label_style()),
        Span::raw(state.project_root.display().to_string()),
    ]);
    frame.render_widget(Paragraph::new(vec![footer]), area);
}

fn render_permission_overlay(frame: &mut Frame<'_>, area: Rect, pending: &PendingPermission) {
    let width = area.width.saturating_sub(8).clamp(44, 96);
    let height = 9.min(area.height.saturating_sub(4)).max(7);
    let rect = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    };
    let prompt = strip_ansi(&crate::repl::format_permission_prompt_line_for_sink(
        &pending.request,
        &pending.host_role,
        usize::from(width.saturating_sub(4)),
    ));
    let lines = vec![
        Line::from(vec![Span::styled(
            format!(
                "@{} requests {}",
                pending.request.role, pending.request.tool
            ),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::raw(""),
        Line::raw(prompt),
        Line::raw(""),
        Line::from(vec![
            Span::styled(
                "a",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" allow once   "),
            Span::styled(
                "s",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" allow session   "),
            Span::styled(
                "d",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" deny   "),
            Span::styled(
                "n/esc",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" never"),
        ]),
    ];
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title("Permission"))
            .wrap(Wrap { trim: true }),
        rect,
    );
}

fn label_style() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}

fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            for inner in chars.by_ref() {
                if inner.is_ascii_alphabetic() {
                    break;
                }
            }
        } else if ch != '\r' {
            out.push(ch);
        }
    }
    out
}

fn buffer_to_string(buffer: &Buffer) -> String {
    let mut lines = Vec::new();
    for y in 0..buffer.area.height {
        let mut line = String::new();
        for x in 0..buffer.area.width {
            line.push_str(buffer[(x, y)].symbol());
        }
        lines.push(line.trim_end().to_owned());
    }
    lines.join("\n")
}

fn write_enter_commands<W: Write>(mut writer: W) -> io::Result<()> {
    execute!(writer, EnterAlternateScreen, Hide, EnableBracketedPaste)
}

fn write_leave_commands<W: Write>(mut writer: W) -> io::Result<()> {
    execute!(writer, DisableBracketedPaste, Show, LeaveAlternateScreen)
}

#[derive(Debug)]
struct RoomTerminalGuard;

impl RoomTerminalGuard {
    fn enter() -> Result<Self> {
        terminal::enable_raw_mode().context("enable live room raw mode")?;
        if let Err(error) = write_enter_commands(io::stdout()) {
            let _ = terminal::disable_raw_mode();
            return Err(error).context("enter live room alternate screen");
        }
        Ok(Self)
    }
}

impl Drop for RoomTerminalGuard {
    fn drop(&mut self) {
        let _ = write_leave_commands(io::stdout());
        let _ = terminal::disable_raw_mode();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::work_card::{Step, StepKind};
    use crossterm::style::Color as CrosstermColor;
    use serde_json::json;
    use std::time::Instant;

    #[test]
    fn render_runtime_frame_contains_composer_and_cursor_surface() {
        let mut state = test_state();
        state.composer.insert_char('@');
        state.composer.insert_char('h');
        let text = render_room_runtime_to_text(&state, 100, 28).expect("render");
        assert!(text.contains("CoreRoom Runtime"));
        assert!(text.contains("cr > @h"));
        assert!(text.contains("Ask @host"));
    }

    #[test]
    fn permission_prompt_blocks_composer_and_renders_overlay() {
        let mut state = test_state();
        let (tx, mut rx) = mpsc::unbounded_channel();
        state.apply_event(RoomEvent::PermissionPrompt {
            request: sample_request(),
            host_role: "host".to_owned(),
            response_tx: Some(tx),
        });
        assert_eq!(
            state.composer.submission_state(),
            ComposerSubmissionState::Blocked
        );
        let text = render_room_runtime_to_text(&state, 100, 28).expect("render");
        assert!(text.contains("Permission"));
        assert!(text.contains("@backend requests Bash"));

        answer_permission(
            &mut state,
            permission_decision(PermissionDecision::Allow, DecisionScope::Once),
        );
        let response = rx.try_recv().expect("permission response sent");
        assert_eq!(response.decision, PermissionDecision::Allow);
    }

    #[test]
    fn work_and_spinner_events_populate_status_rail() {
        let mut state = test_state();
        state.apply_event(RoomEvent::Spinner(SpinnerSnapshot {
            role: "backend".to_owned(),
            frame: 1,
            started_at: Instant::now(),
            tools_seen: 0,
            current_state: Some("thinking".to_owned()),
            paint: SpinnerPaint::Painting,
        }));
        state.apply_event(RoomEvent::WorkCard(sample_work_card()));
        let text = render_room_runtime_to_text(&state, 120, 30).expect("render");
        assert!(text.contains("@backend"));
        assert!(text.contains("Run validation"));
    }

    fn test_state() -> RoomRuntimeState {
        RoomRuntimeState::new(
            PathBuf::from("/tmp/CoreRoom"),
            "host".to_owned(),
            vec!["host".to_owned(), "backend".to_owned()],
        )
    }

    fn sample_request() -> BridgeRequest {
        BridgeRequest {
            v: 1,
            role: "backend".to_owned(),
            tool: "Bash".to_owned(),
            input: json!({"command": "cargo test"}),
            reason: "ask".to_owned(),
        }
    }

    fn sample_work_card() -> WorkCard {
        WorkCard {
            role: "backend".to_owned(),
            host_role: "host".to_owned(),
            role_color: CrosstermColor::Cyan,
            title: "Run validation".to_owned(),
            status: WorkStatus::Working {
                spinner_frame: 0,
                current_step: Some("cargo test".to_owned()),
            },
            steps: vec![Step {
                kind: StepKind::Active,
                text: "cargo test".to_owned(),
                time: None,
            }],
            collapsed: false,
        }
    }
}
