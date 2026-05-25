//! Full-screen ratatui host for the executable CoreRoom runtime.
//!
//! This surface is not a snapshot viewer. It feeds submitted composer
//! lines into the existing REPL parser and renders the same `RoomEvent`
//! stream that `cr start` writes to stdout.

use std::collections::BTreeMap;
use std::io::{self, IsTerminal as _, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

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

use crate::adapter::Engine;
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
use crate::tui_style;

/// Mutable render state for the executable room.
#[derive(Debug)]
pub struct RoomRuntimeState {
    project_root: PathBuf,
    project_name: String,
    host_role: String,
    team: Vec<TeamMember>,
    last_seen: BTreeMap<String, Instant>,
    composer: ComposerState,
    scrollback: Vec<Line<'static>>,
    /// Role of the most recent `RoleSpoke` / `RoleOutputDelta` event
    /// pushed into scrollback. Used to suppress the redundant
    /// `  @role` divider line that the markdown renderer emits at the
    /// top of every chunk when consecutive chunks come from the same
    /// speaker.
    last_speaker: Option<String>,
    spinners: BTreeMap<String, SpinnerSnapshot>,
    work_cards: BTreeMap<String, WorkCard>,
    permission: Option<PendingPermission>,
    exiting: bool,
    show_cheatsheet: bool,
}

/// One row in the right-rail Team roster. Holds only the identity bits
/// the rail needs to render an idle row; live work state lives in
/// [`RoomRuntimeState::spinners`] and [`RoomRuntimeState::work_cards`].
#[derive(Debug, Clone)]
pub struct TeamMember {
    /// Role name without the leading `@`.
    pub role: String,
    /// Engine that this role's adapter targets.
    pub engine: Engine,
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
        let team = build_team(cfg.as_ref(), &host_role);
        let roles: Vec<String> = team.iter().map(|member| member.role.clone()).collect();
        Self::new(project_root.to_path_buf(), host_role, roles, team)
    }

    fn new(
        project_root: PathBuf,
        host_role: String,
        roles: Vec<String>,
        team: Vec<TeamMember>,
    ) -> Self {
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
            team,
            last_seen: BTreeMap::new(),
            composer,
            scrollback: Vec::new(),
            last_speaker: None,
            spinners: BTreeMap::new(),
            work_cards: BTreeMap::new(),
            permission: None,
            exiting: false,
            show_cheatsheet: false,
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
                let speaker = speaker_of(event.as_ref());
                let rendered = StdoutSink::render_to_string(&RoomEvent::Crep {
                    event: event.clone(),
                    host_role: host_role.clone(),
                });
                let cleaned = match (&speaker, &self.last_speaker) {
                    (Some(role), Some(prev)) if role == prev => {
                        strip_leading_role_header(&rendered, role)
                    }
                    _ => rendered,
                };
                self.push_rendered(&cleaned);
                // Track the speaker so the next chunk from the same
                // role can suppress its redundant header.
                if let Some(role) = speaker {
                    self.last_speaker = Some(role);
                }
                if ends_turn {
                    self.last_speaker = None;
                    if !self.has_active_work() {
                        self.composer
                            .set_submission_state(ComposerSubmissionState::Idle);
                    }
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
                self.last_seen.insert(snapshot.role.clone(), Instant::now());
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
        // Style the @user tag consistently with the colored role
        // identities around it. Off-white bold reads as "you" without
        // colliding with any role slot in the palette.
        self.push_scrollback(Line::from(vec![
            Span::styled(
                "@user".to_owned(),
                Style::default()
                    .fg(USER_TAG_COLOR)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(" {line}")),
        ]));
    }

    fn push_notice(&mut self, level: NoticeLevel, text: impl Into<String>) {
        let (label, color) = match level {
            NoticeLevel::Ok => ("ok", Color::Green),
            NoticeLevel::Warn => ("warn", Color::Yellow),
            NoticeLevel::Bad => ("error", Color::Red),
            NoticeLevel::Hint => ("hint", Color::Gray),
            NoticeLevel::System => ("system", Color::DarkGray),
        };
        let body = text.into();
        let line = Line::from(vec![
            Span::styled(
                format!("{label}: "),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::raw(body),
        ]);
        self.push_scrollback(line);
    }

    fn push_rendered(&mut self, text: &str) {
        // Preserve crossterm SGR colors emitted by the splash, CREP
        // renderer, and permission-outcome formatter. Stripping ANSI
        // here would leave role identity, frame strokes, and status
        // colors as plain gray text inside the live-room.
        for line in crate::ansi::ansi_to_lines(text) {
            self.push_scrollback(line);
        }
    }

    fn push_scrollback(&mut self, line: Line<'static>) {
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

    /// Number of distinct active roles. A role that has both a live
    /// spinner snapshot and a working work card (the common case for a
    /// turn in flight) is counted exactly once.
    fn active_work_count(&self) -> usize {
        let mut roles: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for role in self.spinners.keys() {
            roles.insert(role.as_str());
        }
        for card in self.work_cards.values() {
            if matches!(card.status, WorkStatus::Working { .. }) {
                roles.insert(card.role.as_str());
            }
        }
        roles.len()
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
    // The TUI's top status bar and Room panel already provide the
    // identity title and outer border; ask the splash to render
    // frameless so we do not duplicate them.
    let mut runtime_options = options;
    runtime_options.frameless_splash = true;
    let runtime_task = tokio::spawn(async move {
        crate::repl::run_with_options_and_sink_input(&runtime_root, runtime_options, sink, input_rx)
            .await
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
    if state.show_cheatsheet {
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('?')) {
            state.show_cheatsheet = false;
        }
        return Ok(());
    }
    if state.permission.is_some() {
        if let Some(response) = permission_response_for_key(key) {
            answer_permission(state, response);
        }
        return Ok(());
    }

    match key.code {
        KeyCode::Char('?')
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                && state.composer.view_model().input.is_empty() =>
        {
            state.show_cheatsheet = true;
        }
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
            // Esc has three jobs depending on what's open:
            //   1. dismiss an active completion candidate
            //   2. otherwise, clear the input buffer
            //   3. otherwise, noop
            if state.composer.view_model().ghost_suffix.is_some()
                || !state.composer.view_model().candidates.is_empty()
            {
                let _ = state.composer.dismiss_completion();
            } else if !state.composer.view_model().input.is_empty() {
                state.composer.clear();
            }
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
            Constraint::Length(1),
            Constraint::Min(8),
            Constraint::Length(5),
            Constraint::Length(1),
        ])
        .split(area);

    render_status_bar(frame, root[0], state);
    render_body(frame, root[1], state);
    render_composer(frame, root[2], state);
    render_footer(frame, root[3], state);
    if let Some(permission) = &state.permission {
        render_permission_overlay(frame, area, permission);
    } else if state.show_cheatsheet {
        render_cheatsheet_overlay(frame, area, state);
    }
}

/// Top chrome row: a single borderless line carrying identity on the
/// left (product + version + project + short path) and runtime state
/// on the right (status badge with color, `work N` when N > 0).
///
/// At narrow widths the path truncates with `…` and the status badge
/// degrades to text-only; identity (product + version + project) is
/// never dropped.
fn render_status_bar(frame: &mut Frame<'_>, area: Rect, state: &RoomRuntimeState) {
    let runtime_state = current_status(state);
    let badge_label = runtime_state.label();
    let badge_color = runtime_state.color();
    let work = state.active_work_count();
    let work_text = if work > 0 {
        format!("  work {work}")
    } else {
        String::new()
    };

    let identity_full = format!(
        "CoreRoom v{}  ·  {}  ·  {}",
        env!("CARGO_PKG_VERSION"),
        state.project_name,
        home_relative(&state.project_root)
    );

    let right_visible = badge_label.chars().count() + 2 + work_text.chars().count();
    let total = area.width as usize;
    let identity_budget = total.saturating_sub(right_visible).saturating_sub(2).max(1);
    let identity_truncated = truncate_with_ellipsis(&identity_full, identity_budget);
    let identity_visible = identity_truncated.chars().count();

    // Spacer fills any gap between identity (left) and badge (right).
    let spacer_width = total
        .saturating_sub(identity_visible)
        .saturating_sub(right_visible);
    let spacer = " ".repeat(spacer_width);

    let line = Line::from(vec![
        Span::styled(identity_truncated, Style::default().fg(Color::Gray)),
        Span::raw(spacer),
        Span::styled(
            "●",
            Style::default()
                .fg(badge_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            badge_label.to_owned(),
            Style::default()
                .fg(badge_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(work_text, Style::default().fg(Color::DarkGray)),
    ]);
    frame.render_widget(Paragraph::new(vec![line]), area);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeStatus {
    Idle,
    Working,
    WaitingApproval,
    Exiting,
}

impl RuntimeStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Working => "working",
            Self::WaitingApproval => "waiting approval",
            Self::Exiting => "exiting",
        }
    }

    fn color(self) -> Color {
        match self {
            Self::Idle => Color::DarkGray,
            Self::Working => Color::Green,
            Self::WaitingApproval => Color::Yellow,
            Self::Exiting => Color::Red,
        }
    }
}

fn current_status(state: &RoomRuntimeState) -> RuntimeStatus {
    if state.exiting {
        RuntimeStatus::Exiting
    } else if state.permission.is_some() {
        RuntimeStatus::WaitingApproval
    } else if state.has_active_work() {
        RuntimeStatus::Working
    } else {
        RuntimeStatus::Idle
    }
}

fn home_relative(path: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(rel) = path.strip_prefix(&home) {
            let rel_str = rel.display().to_string();
            return if rel_str.is_empty() {
                "~".to_owned()
            } else {
                format!("~/{rel_str}")
            };
        }
    }
    path.display().to_string()
}

fn truncate_with_ellipsis(input: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    if input.chars().count() <= max_chars {
        return input.to_owned();
    }
    let mut out: String = input.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
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
            .map(|line| ListItem::new(line.clone()))
            .collect()
    };
    frame.render_widget(
        List::new(items).block(Block::default().borders(Borders::ALL).title("Room")),
        area,
    );
}

fn render_status_rail(frame: &mut Frame<'_>, area: Rect, state: &RoomRuntimeState) {
    // The Work panel only appears when there is something to show.
    // Otherwise the Team/Roles panel takes the full rail height. Rail
    // width is constant, so the swap never causes horizontal jitter.
    if state.work_cards.is_empty() {
        render_role_lane(frame, area, state);
    } else {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(6), Constraint::Length(10)])
            .split(area);
        render_role_lane(frame, chunks[0], state);
        render_work_cards(frame, chunks[1], state);
    }
}

fn render_role_lane(frame: &mut Frame<'_>, area: Rect, state: &RoomRuntimeState) {
    let mut lines = Vec::new();
    let pending_role = state.permission.as_ref().map(|p| p.request.role.as_str());
    let pending_active = pending_role.is_some()
        && pending_role.is_some_and(|role| !state.spinners.contains_key(role));
    let title;
    if state.spinners.is_empty() && !pending_active {
        title = "Team";
        let now = Instant::now();
        for member in &state.team {
            lines.push(team_line(member, &state.host_role, &state.last_seen, now));
        }
    } else {
        title = "Roles";
        for snapshot in state.spinners.values() {
            lines.push(spinner_line(snapshot, &state.host_role));
        }
        // If a permission overlay is open for a role that no longer has
        // a live spinner (because the kernel cleared it before/while
        // the prompt arrived), synthesize a `waiting approval` row so
        // the rail and the modal point at the same role.
        if let Some(role) = pending_role {
            if !state.spinners.contains_key(role) {
                lines.push(waiting_approval_line(role, &state.host_role));
            }
        }
        let visible_active = state.spinners.len() + usize::from(pending_active);
        let inactive = state.team.len().saturating_sub(visible_active);
        if inactive > 0 {
            lines.push(Line::from(vec![Span::styled(
                format!("+ {inactive} standby"),
                Style::default().fg(Color::DarkGray),
            )]));
        }
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: true }),
        area,
    );
}

/// Synthesised row for a role that is blocked on a permission prompt
/// but has no live spinner of its own. Uses the same yellow paint as
/// [`SpinnerPaint::WaitingApproval`] and keeps the role identity color
/// on the glyph and `@role` token so the rail stays coherent with the
/// permission overlay above it.
fn waiting_approval_line(role: &str, host_role: &str) -> Line<'static> {
    let role_color = tui_style::role_color(role, host_role);
    let glyph = tui_style::role_avatar_glyph(role, host_role);
    Line::from(vec![
        Span::styled(glyph.to_owned(), Style::default().fg(role_color)),
        Span::raw(" "),
        Span::styled("⏸", Style::default().fg(Color::Yellow)),
        Span::raw(" "),
        Span::styled(
            format!("@{role}"),
            Style::default().fg(role_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" · waiting approval", Style::default().fg(Color::Yellow)),
    ])
}

/// One row in the Team roster. Identity (glyph + role token) uses
/// [`tui_style::role_label_spans`]; engine label and last-seen hint
/// trail in dim text. `standby` is the literal hint for any role that
/// has not been seen this session.
fn team_line(
    member: &TeamMember,
    host_role: &str,
    last_seen: &BTreeMap<String, Instant>,
    now: Instant,
) -> Line<'static> {
    let mut spans = tui_style::role_label_spans(&member.role, host_role);
    spans.push(Span::raw("  "));
    spans.push(Span::styled(
        engine_short(member.engine).to_owned(),
        Style::default().fg(Color::Gray),
    ));
    spans.push(Span::raw("  "));
    let hint = last_seen
        .get(&member.role)
        .map_or_else(|| "standby".to_owned(), |seen| last_seen_text(now, *seen));
    spans.push(Span::styled(hint, Style::default().fg(Color::DarkGray)));
    Line::from(spans)
}

fn engine_short(engine: Engine) -> &'static str {
    match engine {
        Engine::Cc => "cc",
        Engine::Codex => "codex",
        Engine::Gemini => "gemini",
        Engine::Fake => "fake",
    }
}

/// Compact session-relative timestamp for the Team roster. Best-effort
/// from [`Instant`]; not persisted across sessions.
fn last_seen_text(now: Instant, seen: Instant) -> String {
    let elapsed = now.saturating_duration_since(seen).as_secs();
    match elapsed {
        0 => "active".to_owned(),
        s if s < 60 => format!("{s}s ago"),
        s if s < 3600 => format!("{}m ago", s / 60),
        s => format!("{}h ago", s / 3600),
    }
}

/// Build the Team roster: host first, then declared roles in
/// alphabetical order with the host excluded from the tail. Falls back
/// to a single host row when no config is available.
fn build_team(cfg: Option<&Config>, host_role: &str) -> Vec<TeamMember> {
    let Some(cfg) = cfg else {
        return vec![TeamMember {
            role: host_role.to_owned(),
            engine: Engine::Cc,
        }];
    };
    let mut others: Vec<TeamMember> = cfg
        .role_names()
        .filter(|name| *name != host_role)
        .map(|name| {
            let engine = cfg
                .roles
                .get(name)
                .and_then(|entry| entry.engine)
                .unwrap_or(cfg.default_engine);
            TeamMember {
                role: name.to_owned(),
                engine,
            }
        })
        .collect();
    others.sort_by(|a, b| a.role.cmp(&b.role));

    let host_engine = cfg
        .roles
        .get(host_role)
        .and_then(|entry| entry.engine)
        .unwrap_or(cfg.default_engine);
    let mut team = Vec::with_capacity(others.len() + 1);
    team.push(TeamMember {
        role: host_role.to_owned(),
        engine: host_engine,
    });
    team.extend(others);
    team
}

const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Role that a CREP event represents as a "speaker". Used to dedupe
/// consecutive headers from the same role.
fn speaker_of(event: &CrepEvent) -> Option<String> {
    match event {
        CrepEvent::RoleSpoke { role, .. } | CrepEvent::RoleOutputDelta { role, .. } => {
            Some(role.clone())
        }
        _ => None,
    }
}

/// Strip the leading `  @role` header line that
/// `render_role_markdown` emits at the top of every chunk. Used when
/// the previous push to scrollback was already a chunk from the same
/// role, so the header would just repeat.
fn strip_leading_role_header(rendered: &str, role: &str) -> String {
    let Some(newline) = rendered.find('\n') else {
        return rendered.to_owned();
    };
    let first = &rendered[..newline];
    let plain = strip_ansi(first);
    let trimmed = plain.trim();
    let role_token = format!("@{role}");
    // The header is just the role token (optionally followed by a
    // single space + suffix). Any actual body line either starts with
    // four spaces of body_prefix or contains characters beyond the
    // role token.
    if trimmed == role_token || trimmed.starts_with(&format!("{role_token} ")) {
        rendered[newline + 1..].to_owned()
    } else {
        rendered.to_owned()
    }
}

/// Display color for the `@user` tag in scrollback. Picked to read as
/// "you" without collding with any role slot in the palette (host
/// lavender, engineer/backend sky, reviewer blossom, security coral,
/// qa honey, sre teal, frontend rose, product jade). `EM` from the
/// crossterm palette is `RGB(0xf0, 0xf0, 0xf0)` — warm off-white.
const USER_TAG_COLOR: Color = Color::Rgb(0xf0, 0xf0, 0xf0);

fn spinner_line(snapshot: &SpinnerSnapshot, host_role: &str) -> Line<'static> {
    let frame = SPINNER_FRAMES[snapshot.frame % SPINNER_FRAMES.len()];
    let state = snapshot
        .current_state
        .clone()
        .unwrap_or_else(|| "thinking".to_owned());
    let frame_style = match snapshot.paint {
        SpinnerPaint::WaitingApproval => Style::default().fg(Color::Yellow),
        SpinnerPaint::Painting => Style::default().fg(Color::Cyan),
        SpinnerPaint::Cleared => Style::default().fg(Color::DarkGray),
    };
    let role_color = tui_style::role_color(&snapshot.role, host_role);
    let glyph = tui_style::role_avatar_glyph(&snapshot.role, host_role);
    Line::from(vec![
        Span::styled(glyph.to_owned(), Style::default().fg(role_color)),
        Span::raw(" "),
        Span::styled(frame, frame_style),
        Span::raw(" "),
        Span::styled(
            format!("@{}", snapshot.role),
            Style::default().fg(role_color).add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            " · {}s · ",
            snapshot.started_at.elapsed().as_secs()
        )),
        Span::styled(state, Style::default().fg(Color::DarkGray)),
    ])
}

fn render_work_cards(frame: &mut Frame<'_>, area: Rect, state: &RoomRuntimeState) {
    // Caller is responsible for not invoking this when work_cards is
    // empty — the rail folds the panel in that case (see
    // `render_status_rail`). Defensive return keeps the function safe
    // to call from snapshot tests.
    if state.work_cards.is_empty() {
        return;
    }
    let mut lines = Vec::new();
    let width = usize::from(area.width.saturating_sub(4)).max(28);
    for card in state.work_cards.values().rev().take(3) {
        // The card body's border title already carries the colored
        // `@role` token via the v0.9.10 ANSI bridge, so no separate
        // identity header is needed — adding one printed the role
        // mention twice on every card.
        for line in crate::ansi::ansi_to_lines(&card.render(width)) {
            lines.push(line);
        }
        lines.push(Line::raw(""));
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
    let visual = composer_visual(state, &vm);
    let lines = composer_lines(&vm, &state.host_role, &visual);
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(visual.border_style)
                    .title(visual.title.clone()),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
    if vm.submission_state != ComposerSubmissionState::Blocked {
        frame.set_cursor_position(composer_cursor_position(area, &vm, visual.prompt_width));
    }
}

/// Visual state of the composer derived purely from
/// [`ComposerSubmissionState`] and [`RoomRuntimeState::permission`].
/// Three states: idle (default), working (gray border + inline mini
/// spinner), blocked (yellow border + dim body + hint).
#[derive(Debug, Clone)]
struct ComposerVisual {
    title: String,
    border_style: Style,
    prompt_prefix_spans: Vec<Span<'static>>,
    /// Visible columns the prompt prefix occupies. Used by the cursor
    /// position calc; must match the total `chars().count()` of the
    /// prompt spans.
    prompt_width: usize,
    body_style: Option<Style>,
    blocked_hint: Option<&'static str>,
}

fn composer_visual(state: &RoomRuntimeState, vm: &ComposerViewModel) -> ComposerVisual {
    match vm.submission_state {
        ComposerSubmissionState::Blocked => ComposerVisual {
            title: "Permission required".to_owned(),
            border_style: Style::default().fg(Color::Yellow),
            prompt_prefix_spans: vec![Span::styled("cr > ", Style::default().fg(Color::DarkGray))],
            prompt_width: 5,
            body_style: Some(Style::default().fg(Color::DarkGray)),
            blocked_hint: Some("waiting for your approval above"),
        },
        ComposerSubmissionState::Submitting => {
            let frame_idx = state
                .spinners
                .values()
                .next()
                .map_or(0, |snapshot| snapshot.frame % SPINNER_FRAMES.len());
            let mini_spinner = SPINNER_FRAMES[frame_idx];
            let suffix = if vm.input.is_empty() {
                "working"
            } else {
                "queued"
            };
            ComposerVisual {
                title: format!("Ask @{} · {}", state.host_role, suffix),
                border_style: Style::default().fg(Color::DarkGray),
                prompt_prefix_spans: vec![
                    Span::styled(mini_spinner, Style::default().fg(Color::Cyan)),
                    Span::raw(" "),
                    Span::styled("cr > ", Style::default().fg(Color::Green)),
                ],
                // mini spinner (1) + space (1) + "cr > " (5) = 7
                prompt_width: 7,
                body_style: None,
                blocked_hint: None,
            }
        }
        ComposerSubmissionState::Idle => ComposerVisual {
            title: format!("Ask @{}", state.host_role),
            border_style: Style::default(),
            prompt_prefix_spans: vec![Span::styled("cr > ", Style::default().fg(Color::Green))],
            prompt_width: 5,
            body_style: None,
            blocked_hint: None,
        },
    }
}

fn composer_lines(
    vm: &ComposerViewModel,
    host_role: &str,
    visual: &ComposerVisual,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let input = vm.input.clone();
    let continuation = " ".repeat(visual.prompt_width);

    if input.is_empty() {
        let mut spans = visual.prompt_prefix_spans.clone();
        spans.push(Span::styled(
            vm.prompt_hint.clone(),
            Style::default().fg(Color::DarkGray),
        ));
        lines.push(Line::from(spans));
    } else {
        for (index, line) in input.lines().enumerate() {
            let mut spans = Vec::new();
            if index == 0 {
                spans.extend(visual.prompt_prefix_spans.clone());
            } else {
                spans.push(Span::raw(continuation.clone()));
            }
            let body_span = match visual.body_style {
                Some(style) => Span::styled(line.to_owned(), style),
                None => Span::raw(line.to_owned()),
            };
            spans.push(body_span);
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
    if let Some(hint) = visual.blocked_hint {
        lines.push(Line::from(vec![Span::styled(
            hint,
            Style::default().fg(Color::Yellow),
        )]));
    }
    if !vm.candidates.is_empty() {
        let mut spans: Vec<Span<'static>> = Vec::new();
        for (i, candidate) in vm.candidates.iter().take(4).enumerate() {
            if i > 0 {
                spans.push(Span::raw("  "));
            }
            spans.extend(candidate_spans(candidate, host_role));
        }
        lines.push(Line::from(spans));
    }
    lines
}

/// Style one completion candidate. Role mentions (`@role`) pick up the
/// role's identity color; slash commands stay neutral dim gray. The
/// selected candidate is wrapped in `[…]` brackets so a screen-reader
/// or no-color terminal can still tell which one is active.
fn candidate_spans(
    candidate: &crate::console_composer::ComposerCandidate,
    host_role: &str,
) -> Vec<Span<'static>> {
    let label = candidate.label.clone();
    let selected = candidate.selected;
    let role_style = label.strip_prefix('@').map(|role| {
        Style::default()
            .fg(tui_style::role_color(role, host_role))
            .add_modifier(Modifier::BOLD)
    });
    let body_style = role_style.unwrap_or_else(|| Style::default().fg(Color::DarkGray));
    if selected {
        vec![
            Span::styled("[", Style::default().fg(Color::DarkGray)),
            Span::styled(label, body_style),
            Span::styled("]", Style::default().fg(Color::DarkGray)),
        ]
    } else {
        vec![Span::styled(label, body_style)]
    }
}

fn composer_cursor_position(area: Rect, vm: &ComposerViewModel, prompt_width: usize) -> (u16, u16) {
    let inner_x = area.x.saturating_add(1);
    let inner_y = area.y.saturating_add(1);
    let inner_width = area.width.saturating_sub(2);
    let inner_height = area.height.saturating_sub(2).max(1);
    let (row, col) = cursor_row_col(&vm.input, vm.cursor);
    let prompt_width = u16::try_from(prompt_width).unwrap_or(u16::MAX);
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
    let bindings = footer_bindings(state);
    let fitted = fit_bindings_to_width(&bindings, area.width.into());
    frame.render_widget(Paragraph::new(vec![bindings_to_line(&fitted)]), area);
}

/// One footer chip: a styled keys badge + a plain action label.
/// Priority controls drop order when the row is wider than the area:
/// lower priorities (0) are never dropped; higher priorities go first.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FooterBinding {
    keys: &'static str,
    action: &'static str,
    /// 0 = primary action / halt / cancel (never dropped).
    /// 1 = destructive (ctrl-d exit).
    /// 2 = `? help`.
    /// 3 = secondary actions (newline, clear).
    priority: u8,
    /// `true` to render the keys with the primary `enter`-style chip
    /// (black-on-cyan); `false` for the bold-yellow label chip.
    primary_chip: bool,
}

fn footer_bindings(state: &RoomRuntimeState) -> Vec<FooterBinding> {
    let typed = !state.composer.view_model().input.is_empty();
    if state.permission.is_some() {
        return vec![
            FooterBinding {
                keys: "y",
                action: "allow",
                priority: 0,
                primary_chip: true,
            },
            FooterBinding {
                keys: "n",
                action: "deny",
                priority: 0,
                primary_chip: false,
            },
            FooterBinding {
                keys: "esc",
                action: "cancel",
                priority: 0,
                primary_chip: false,
            },
        ];
    }
    let working = state.has_active_work();
    let mut bindings = Vec::new();
    if typed {
        bindings.push(FooterBinding {
            keys: "enter",
            action: "send",
            priority: 0,
            primary_chip: true,
        });
    }
    if working {
        bindings.push(FooterBinding {
            keys: "ctrl-c",
            action: "halt",
            priority: 0,
            primary_chip: false,
        });
    }
    if typed {
        bindings.push(FooterBinding {
            keys: "shift+enter",
            action: "newline",
            priority: 3,
            primary_chip: false,
        });
        bindings.push(FooterBinding {
            keys: "esc",
            action: "clear",
            priority: 3,
            primary_chip: false,
        });
    }
    bindings.push(FooterBinding {
        keys: "?",
        action: "help",
        priority: 2,
        primary_chip: false,
    });
    if !typed && !working {
        bindings.push(FooterBinding {
            keys: "ctrl-d",
            action: "exit",
            priority: 1,
            primary_chip: false,
        });
    }
    bindings
}

/// Drop highest-priority bindings until the row fits `max_width`.
/// Priorities 0 (primary / halt / cancel) are never dropped.
fn fit_bindings_to_width(bindings: &[FooterBinding], max_width: usize) -> Vec<FooterBinding> {
    let mut current: Vec<FooterBinding> = bindings.to_vec();
    while bindings_render_width(&current) > max_width {
        let drop_index = current
            .iter()
            .enumerate()
            .filter(|(_, b)| b.priority > 0)
            .max_by_key(|(_, b)| b.priority)
            .map(|(i, _)| i);
        match drop_index {
            Some(i) => {
                current.remove(i);
            }
            None => break,
        }
    }
    current
}

fn binding_visible_width(b: &FooterBinding) -> usize {
    // ` <keys> ` plus a space plus `<action>`
    1 + b.keys.chars().count() + 1 + 1 + b.action.chars().count()
}

fn bindings_render_width(bindings: &[FooterBinding]) -> usize {
    let mut total = 0;
    for (i, b) in bindings.iter().enumerate() {
        if i > 0 {
            total += 2; // "  " separator between chips
        }
        total += binding_visible_width(b);
    }
    total
}

fn bindings_to_line(bindings: &[FooterBinding]) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, b) in bindings.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        let chip_style = if b.primary_chip {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            label_style()
        };
        spans.push(Span::styled(format!(" {} ", b.keys), chip_style));
        spans.push(Span::raw(format!(" {}", b.action)));
    }
    Line::from(spans)
}

/// Centered help overlay listing every binding available in the current
/// state. Triggered by `?` in idle/working states and dismissed by `?`
/// or `Esc`. Hidden in permission-blocked state because the permission
/// modal carries its own key list.
fn render_cheatsheet_overlay(frame: &mut Frame<'_>, area: Rect, state: &RoomRuntimeState) {
    let bindings = cheatsheet_bindings(state);
    let inner_w = bindings
        .iter()
        .map(|b| 4 + b.keys.chars().count() + b.action.chars().count())
        .max()
        .unwrap_or(40);
    let width = u16::try_from(inner_w + 4)
        .unwrap_or(60)
        .clamp(40, area.width.saturating_sub(4).max(40));
    // header (1) + empty (1) + bindings (N) + empty (1) + close (1)
    // + top/bottom borders (2) = N + 6.
    let height = u16::try_from(bindings.len() + 6)
        .unwrap_or(14)
        .clamp(8, area.height.saturating_sub(4));
    let rect = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    };
    let mut lines = vec![
        Line::from(vec![Span::styled(
            "Keys for this state",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::raw(""),
    ];
    for b in &bindings {
        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", b.keys), label_style()),
            Span::raw(format!("  {}", b.action)),
        ]));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![Span::styled(
        "? / esc to close",
        Style::default().fg(Color::DarkGray),
    )]));
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title("Help"))
            .wrap(Wrap { trim: false }),
        rect,
    );
}

/// Full key list shown inside the cheatsheet overlay. Includes the
/// bindings dropped by the contextual footer so users can still
/// discover them.
fn cheatsheet_bindings(state: &RoomRuntimeState) -> Vec<FooterBinding> {
    let typed = !state.composer.view_model().input.is_empty();
    let working = state.has_active_work();
    let mut bindings = Vec::new();
    if state.permission.is_some() {
        bindings.push(FooterBinding {
            keys: "y / a",
            action: "allow once",
            priority: 0,
            primary_chip: false,
        });
        bindings.push(FooterBinding {
            keys: "s",
            action: "allow session",
            priority: 0,
            primary_chip: false,
        });
        bindings.push(FooterBinding {
            keys: "d",
            action: "deny once",
            priority: 0,
            primary_chip: false,
        });
        bindings.push(FooterBinding {
            keys: "n / esc",
            action: "deny session",
            priority: 0,
            primary_chip: false,
        });
        return bindings;
    }
    bindings.push(FooterBinding {
        keys: "enter",
        action: if typed { "send" } else { "send (type first)" },
        priority: 0,
        primary_chip: true,
    });
    bindings.push(FooterBinding {
        keys: "shift+enter",
        action: "newline",
        priority: 3,
        primary_chip: false,
    });
    bindings.push(FooterBinding {
        keys: "tab",
        action: "next completion",
        priority: 3,
        primary_chip: false,
    });
    bindings.push(FooterBinding {
        keys: "esc",
        action: "dismiss / clear",
        priority: 3,
        primary_chip: false,
    });
    if working {
        bindings.push(FooterBinding {
            keys: "ctrl-c",
            action: "halt active turn",
            priority: 0,
            primary_chip: false,
        });
    }
    bindings.push(FooterBinding {
        keys: "ctrl-d",
        action: "exit room",
        priority: 1,
        primary_chip: false,
    });
    bindings.push(FooterBinding {
        keys: "?",
        action: "toggle this help",
        priority: 2,
        primary_chip: false,
    });
    bindings
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
        // Top status row carries identity (product + version + project).
        assert!(text.contains(concat!("CoreRoom v", env!("CARGO_PKG_VERSION"))));
        assert!(text.contains("CoreRoom"));
        // Idle status badge text appears once on the boot frame.
        assert!(text.contains("idle"));
        assert!(text.contains("cr > @h"));
        assert!(text.contains("Ask @host"));
    }

    #[test]
    fn status_bar_replaces_bordered_runtime_header() {
        let state = test_state();
        let text = render_room_runtime_to_text(&state, 100, 28).expect("render");
        // The old bordered title is gone.
        assert!(!text.contains("CoreRoom Runtime"));
        // Identity material renders once.
        let version_token = concat!("v", env!("CARGO_PKG_VERSION"));
        assert_eq!(text.matches(version_token).count(), 1);
    }

    #[test]
    fn status_bar_shows_work_count_only_when_active() {
        let mut state = test_state();
        let text_idle = render_room_runtime_to_text(&state, 100, 28).expect("render");
        assert!(!text_idle.contains("work 1"));
        assert!(!text_idle.contains("work 0"));
        state.apply_event(RoomEvent::Spinner(SpinnerSnapshot {
            role: "backend".to_owned(),
            frame: 0,
            started_at: Instant::now(),
            tools_seen: 0,
            current_state: None,
            paint: SpinnerPaint::Painting,
        }));
        let text_working = render_room_runtime_to_text(&state, 120, 28).expect("render");
        assert!(text_working.contains("working"));
        assert!(text_working.contains("work 1"));
    }

    #[test]
    fn rail_shows_team_roster_when_no_spinners_and_no_cards() {
        let state = test_state();
        let text = render_room_runtime_to_text(&state, 120, 30).expect("render");
        // Roster title is "Team", not "Roles", and the placeholder
        // strings from the previous behavior are gone.
        assert!(text.contains("Team"));
        assert!(!text.contains("no work cards yet"));
        // Roster lists both host and backend, host appears first.
        let host_idx = text.find("@host").expect("host in roster");
        let backend_idx = text.find("@backend").expect("backend in roster");
        assert!(host_idx < backend_idx, "host should appear before backend");
        // Standby hint for roles that have not been seen this session.
        assert!(text.contains("standby"));
    }

    #[test]
    fn rail_shows_active_roles_and_standby_tail_when_spinning() {
        let mut state = test_state();
        state.apply_event(RoomEvent::Spinner(SpinnerSnapshot {
            role: "backend".to_owned(),
            frame: 0,
            started_at: Instant::now(),
            tools_seen: 0,
            current_state: Some("thinking".to_owned()),
            paint: SpinnerPaint::Painting,
        }));
        let text = render_room_runtime_to_text(&state, 120, 30).expect("render");
        // Active panel uses the legacy "Roles" title.
        assert!(text.contains("Roles"));
        // Inactive roles fold into a dim tail.
        assert!(text.contains("+ 1 standby"));
        // The literal `idle` placeholder is gone.
        // (Status badge says `idle` on the chrome row, but the rail
        // panel itself does not.)
        assert!(text.contains("@backend"));
        assert!(text.contains("thinking"));
    }

    #[test]
    fn rail_folds_work_panel_when_no_cards() {
        let state = test_state();
        let text = render_room_runtime_to_text(&state, 120, 30).expect("render");
        // No Work title and no placeholder copy when there are no
        // cards. The Team panel takes the full rail height.
        assert!(!text.contains("Work─"));
        assert!(!text.contains("no work cards yet"));
    }

    #[test]
    fn rail_renders_team_plus_work_when_card_present_but_no_spinner() {
        let mut state = test_state();
        state.apply_event(RoomEvent::WorkCard(sample_work_card()));
        let text = render_room_runtime_to_text(&state, 120, 30).expect("render");
        assert!(text.contains("Team"));
        // Work panel appears with its identity header for the role.
        assert!(text.contains("◇ @backend"));
        assert!(text.contains("Run validation"));
    }

    #[test]
    fn composer_idle_state_uses_default_border_and_green_prompt() {
        let state = test_state();
        let vm = state.composer.view_model();
        let visual = super::composer_visual(&state, &vm);
        assert_eq!(visual.title, "Ask @host");
        assert_eq!(visual.border_style, Style::default());
        assert!(visual.blocked_hint.is_none());
        assert_eq!(visual.prompt_width, 5);
        // First prompt span is `cr > ` in green.
        let prompt_text: String = visual
            .prompt_prefix_spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(prompt_text, "cr > ");
        let last_fg = visual.prompt_prefix_spans.last().unwrap().style.fg;
        assert_eq!(last_fg, Some(Color::Green));
    }

    #[test]
    fn composer_working_state_dims_border_and_prepends_mini_spinner() {
        let mut state = test_state();
        state.apply_event(RoomEvent::Spinner(SpinnerSnapshot {
            role: "backend".to_owned(),
            frame: 3,
            started_at: Instant::now(),
            tools_seen: 0,
            current_state: Some("thinking".to_owned()),
            paint: SpinnerPaint::Painting,
        }));
        let vm = state.composer.view_model();
        let visual = super::composer_visual(&state, &vm);
        assert_eq!(visual.title, "Ask @host · working");
        assert_eq!(visual.border_style.fg, Some(Color::DarkGray));
        // Prompt prefix is `<spinner> cr > `.
        let prompt_text: String = visual
            .prompt_prefix_spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert!(SPINNER_FRAMES.contains(&prompt_text.chars().next().unwrap().to_string().as_str()));
        assert!(prompt_text.ends_with("cr > "));
        assert_eq!(visual.prompt_width, 7);
    }

    #[test]
    fn composer_working_state_with_typed_input_shows_queued_in_title() {
        let mut state = test_state();
        state.apply_event(RoomEvent::Spinner(SpinnerSnapshot {
            role: "backend".to_owned(),
            frame: 0,
            started_at: Instant::now(),
            tools_seen: 0,
            current_state: None,
            paint: SpinnerPaint::Painting,
        }));
        state.composer.insert_char('h');
        state.composer.insert_char('i');
        let vm = state.composer.view_model();
        let visual = super::composer_visual(&state, &vm);
        assert_eq!(visual.title, "Ask @host · queued");
    }

    #[test]
    fn composer_blocked_state_yellow_border_and_hint_line() {
        let mut state = test_state();
        let (tx, _rx) = mpsc::unbounded_channel();
        state.apply_event(RoomEvent::PermissionPrompt {
            request: sample_request(),
            host_role: "host".to_owned(),
            response_tx: Some(tx),
        });
        let vm = state.composer.view_model();
        let visual = super::composer_visual(&state, &vm);
        assert_eq!(visual.title, "Permission required");
        assert_eq!(visual.border_style.fg, Some(Color::Yellow));
        assert_eq!(visual.blocked_hint, Some("waiting for your approval above"));
        // Body style dims user input while a permission modal is open.
        assert_eq!(visual.body_style.and_then(|s| s.fg), Some(Color::DarkGray));
    }

    #[test]
    fn composer_blocked_state_renders_yellow_hint_line() {
        let mut state = test_state();
        let (tx, _rx) = mpsc::unbounded_channel();
        state.apply_event(RoomEvent::PermissionPrompt {
            request: sample_request(),
            host_role: "host".to_owned(),
            response_tx: Some(tx),
        });
        let text = render_room_runtime_to_text(&state, 120, 30).expect("render");
        assert!(text.contains("waiting for your approval above"));
        assert!(text.contains("Permission required"));
    }

    #[test]
    fn composer_working_mini_spinner_matches_active_role_frame() {
        let mut state = test_state();
        state.apply_event(RoomEvent::Spinner(SpinnerSnapshot {
            role: "backend".to_owned(),
            frame: 4,
            started_at: Instant::now(),
            tools_seen: 0,
            current_state: None,
            paint: SpinnerPaint::Painting,
        }));
        let vm = state.composer.view_model();
        let visual = super::composer_visual(&state, &vm);
        let glyph = visual.prompt_prefix_spans[0].content.as_ref();
        // Spinner frames are 10-wide; frame 4 ⇒ "⠼".
        assert_eq!(glyph, SPINNER_FRAMES[4]);
    }

    #[test]
    fn footer_no_longer_carries_project_path() {
        let state = test_state();
        let text = render_room_runtime_to_text(&state, 100, 28).expect("render");
        // The project path is identity material and now lives only in
        // the top status row. It must not appear in the footer text.
        assert!(!text.contains(" path "));
        // Project path appears at most once anywhere in the rendered
        // frame (top chrome only).
        let project_path = state.project_root.display().to_string();
        // Truncation can chop characters in the chrome; just assert
        // the literal full path is not duplicated.
        assert!(text.matches(project_path.as_str()).count() <= 1);
    }

    fn binding_pairs(bindings: &[FooterBinding]) -> Vec<(&'static str, &'static str)> {
        bindings.iter().map(|b| (b.keys, b.action)).collect()
    }

    #[test]
    fn footer_idle_empty_shows_help_and_exit_only() {
        let state = test_state();
        let bindings = super::footer_bindings(&state);
        assert_eq!(
            binding_pairs(&bindings),
            vec![("?", "help"), ("ctrl-d", "exit")]
        );
    }

    #[test]
    fn footer_idle_typed_swaps_in_send_and_drops_exit() {
        let mut state = test_state();
        state.composer.insert_char('a');
        let bindings = super::footer_bindings(&state);
        assert_eq!(
            binding_pairs(&bindings),
            vec![
                ("enter", "send"),
                ("shift+enter", "newline"),
                ("esc", "clear"),
                ("?", "help"),
            ]
        );
    }

    #[test]
    fn footer_working_shows_halt_and_help() {
        let mut state = test_state();
        state.apply_event(RoomEvent::Spinner(SpinnerSnapshot {
            role: "backend".to_owned(),
            frame: 0,
            started_at: Instant::now(),
            tools_seen: 0,
            current_state: None,
            paint: SpinnerPaint::Painting,
        }));
        let bindings = super::footer_bindings(&state);
        assert_eq!(
            binding_pairs(&bindings),
            vec![("ctrl-c", "halt"), ("?", "help")]
        );
    }

    #[test]
    fn footer_working_with_typed_input_includes_enter_send() {
        let mut state = test_state();
        state.apply_event(RoomEvent::Spinner(SpinnerSnapshot {
            role: "backend".to_owned(),
            frame: 0,
            started_at: Instant::now(),
            tools_seen: 0,
            current_state: None,
            paint: SpinnerPaint::Painting,
        }));
        state.composer.insert_char('q');
        let bindings = super::footer_bindings(&state);
        assert!(bindings
            .iter()
            .any(|b| b.keys == "enter" && b.action == "send"));
        assert!(bindings.iter().any(|b| b.keys == "ctrl-c"));
    }

    #[test]
    fn footer_blocked_shows_permission_keys_only() {
        let mut state = test_state();
        let (tx, _rx) = mpsc::unbounded_channel();
        state.apply_event(RoomEvent::PermissionPrompt {
            request: sample_request(),
            host_role: "host".to_owned(),
            response_tx: Some(tx),
        });
        let bindings = super::footer_bindings(&state);
        assert_eq!(
            binding_pairs(&bindings),
            vec![("y", "allow"), ("n", "deny"), ("esc", "cancel")]
        );
    }

    #[test]
    fn footer_drops_secondary_actions_first_when_narrow() {
        let mut state = test_state();
        state.composer.insert_char('a');
        let bindings = super::footer_bindings(&state);
        let full_width = super::bindings_render_width(&bindings);
        // Force a width that removes the lowest-priority chip but
        // keeps enter+send and the help chip.
        let fitted = super::fit_bindings_to_width(&bindings, full_width - 4);
        let pairs = binding_pairs(&fitted);
        // The primary chip is never dropped.
        assert!(pairs.contains(&("enter", "send")));
        // A priority-3 binding (newline or clear) has been dropped.
        let priority3_remaining = fitted.iter().filter(|b| b.priority == 3).count();
        assert!(priority3_remaining < 2);
    }

    #[test]
    fn cheatsheet_opens_on_question_mark_and_closes_on_question_mark() {
        let mut state = test_state();
        assert!(!state.show_cheatsheet);
        super::handle_key(
            KeyEvent::new(KeyCode::Char('?'), KeyModifiers::empty()),
            &mut state,
            &mpsc::unbounded_channel().0,
        )
        .unwrap();
        assert!(state.show_cheatsheet);
        super::handle_key(
            KeyEvent::new(KeyCode::Char('?'), KeyModifiers::empty()),
            &mut state,
            &mpsc::unbounded_channel().0,
        )
        .unwrap();
        assert!(!state.show_cheatsheet);
    }

    #[test]
    fn cheatsheet_closes_on_escape() {
        let mut state = test_state();
        state.show_cheatsheet = true;
        super::handle_key(
            KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()),
            &mut state,
            &mpsc::unbounded_channel().0,
        )
        .unwrap();
        assert!(!state.show_cheatsheet);
    }

    #[test]
    fn question_mark_is_typed_when_composer_has_input() {
        let mut state = test_state();
        state.composer.insert_char('h');
        super::handle_key(
            KeyEvent::new(KeyCode::Char('?'), KeyModifiers::empty()),
            &mut state,
            &mpsc::unbounded_channel().0,
        )
        .unwrap();
        assert!(!state.show_cheatsheet);
        assert!(state.composer.view_model().input.contains('?'));
    }

    #[test]
    fn cheatsheet_overlay_renders_when_flag_is_set() {
        let mut state = test_state();
        state.show_cheatsheet = true;
        let text = render_room_runtime_to_text(&state, 120, 30).expect("render");
        assert!(text.contains("Help"));
        assert!(text.contains("toggle this help"));
        assert!(text.contains("? / esc to close"));
    }

    #[test]
    fn banner_with_ansi_preserves_role_colors_in_scrollback() {
        use crossterm::style::Stylize as _;
        let mut state = test_state();
        // Imitate a splash row coming through the sink as a banner.
        // The crossterm-styled string carries 24-bit RGB color escape
        // codes that the ratatui scrollback must keep.
        let role_color = crate::output::role_color("backend", "host");
        let banner = format!("◇ {}  cc · 1M · ask\n", "@backend".with(role_color));
        state.apply_event(RoomEvent::Banner(banner));

        let expected = match role_color {
            crossterm::style::Color::Rgb { r, g, b } => Color::Rgb(r, g, b),
            other => panic!("expected RGB role color, got {other:?}"),
        };
        let coloured = state
            .scrollback
            .iter()
            .flat_map(|line| line.spans.iter())
            .find(|span| span.content.as_ref() == "@backend")
            .expect("@backend span survived scrollback");
        assert_eq!(coloured.style.fg, Some(expected));
    }

    #[test]
    fn work_card_renders_role_label_exactly_once_per_card() {
        let mut state = test_state();
        state.apply_event(RoomEvent::WorkCard(sample_work_card()));
        let text = render_room_runtime_to_text(&state, 120, 30).expect("render");
        // The card's own border title already carries `@backend`. The
        // pre-v0.9.11 identity-header prefix line above the card is
        // gone. Count `@backend` only inside the card body region
        // (the lines between the top and bottom box-drawing borders).
        // Work-card borders use rounded corners (╭ ╰); ratatui panel
        // borders use square corners (┌ └). Match rounded only so the
        // Team panel's `◇ @backend cc standby` row doesn't bleed in.
        let mut inside_card = false;
        let mut card_lines: Vec<&str> = Vec::new();
        for line in text.lines() {
            if line.contains('╭') {
                inside_card = true;
            }
            if inside_card {
                card_lines.push(line);
            }
            if line.contains('╰') {
                inside_card = false;
            }
        }
        let card_section = card_lines.join("\n");
        assert_eq!(
            card_section.matches("@backend").count(),
            1,
            "expected exactly one @backend in card body, got:\n{card_section}"
        );
        // There must be no bare identity-header row above the card.
        let above_card_top = text
            .lines()
            .take_while(|line| !line.contains("Run validation"))
            .filter(|line| line.contains("◇ @backend") && !line.contains("standby"))
            .count();
        assert_eq!(
            above_card_top, 0,
            "expected no `◇ @backend` identity prefix above the card"
        );
    }

    #[test]
    fn active_work_count_treats_spinner_plus_card_as_one_role() {
        let mut state = test_state();
        state.apply_event(RoomEvent::Spinner(SpinnerSnapshot {
            role: "backend".to_owned(),
            frame: 0,
            started_at: Instant::now(),
            tools_seen: 0,
            current_state: None,
            paint: SpinnerPaint::Painting,
        }));
        state.apply_event(RoomEvent::WorkCard(sample_work_card()));
        assert_eq!(state.active_work_count(), 1);
    }

    #[test]
    fn active_work_count_sums_distinct_roles() {
        let mut state = test_state();
        state.apply_event(RoomEvent::Spinner(SpinnerSnapshot {
            role: "host".to_owned(),
            frame: 0,
            started_at: Instant::now(),
            tools_seen: 0,
            current_state: None,
            paint: SpinnerPaint::Painting,
        }));
        state.apply_event(RoomEvent::Spinner(SpinnerSnapshot {
            role: "backend".to_owned(),
            frame: 0,
            started_at: Instant::now(),
            tools_seen: 0,
            current_state: None,
            paint: SpinnerPaint::Painting,
        }));
        assert_eq!(state.active_work_count(), 2);
    }

    #[test]
    fn user_line_is_styled_with_user_identity_color() {
        let mut state = test_state();
        state.push_user_line("write some tests");
        let span = state
            .scrollback
            .iter()
            .flat_map(|line| line.spans.iter())
            .find(|span| span.content.as_ref() == "@user")
            .expect("@user span present");
        assert_eq!(span.style.fg, Some(USER_TAG_COLOR));
        assert!(span.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn permission_overlay_keeps_role_in_roles_rail_without_team_fallback() {
        let mut state = test_state();
        let (tx, _rx) = mpsc::unbounded_channel();
        state.apply_event(RoomEvent::PermissionPrompt {
            request: sample_request(),
            host_role: "host".to_owned(),
            response_tx: Some(tx),
        });
        let text = render_room_runtime_to_text(&state, 120, 30).expect("render");
        // While permission is pending, the rail must show Roles, not
        // the idle Team roster. The requesting role surfaces with a
        // "waiting approval" label.
        assert!(text.contains("Roles"));
        assert!(!text.contains("Team─") && !text.contains("─Team"));
        assert!(text.contains("waiting approval"));
        assert!(text.contains("@backend"));
    }

    #[test]
    fn consecutive_role_chunks_only_render_one_header() {
        let mut state = test_state();
        // Two RoleOutputDelta chunks from the same role.
        let event1 = RoomEvent::Crep {
            event: Box::new(CrepEvent::RoleOutputDelta {
                role: "host".to_owned(),
                priors_hash: String::new(),
                text_delta: "first chunk\n".to_owned(),
                sequence: 1,
                turn_id: String::new(),
                thread_id: String::new(),
            }),
            host_role: "host".to_owned(),
        };
        let event2 = RoomEvent::Crep {
            event: Box::new(CrepEvent::RoleOutputDelta {
                role: "host".to_owned(),
                priors_hash: String::new(),
                text_delta: "second chunk\n".to_owned(),
                sequence: 2,
                turn_id: String::new(),
                thread_id: String::new(),
            }),
            host_role: "host".to_owned(),
        };
        state.apply_event(event1);
        state.apply_event(event2);
        let text = state
            .scrollback
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        // The role token `@host` appears exactly once — the second
        // chunk's header is suppressed.
        assert_eq!(
            text.matches("@host").count(),
            1,
            "expected one @host divider, got:\n{text}"
        );
        assert!(text.contains("first chunk"));
        assert!(text.contains("second chunk"));
    }

    #[test]
    fn role_chunks_from_different_roles_each_keep_their_header() {
        let mut state = test_state();
        state.apply_event(RoomEvent::Crep {
            event: Box::new(CrepEvent::RoleOutputDelta {
                role: "host".to_owned(),
                priors_hash: String::new(),
                text_delta: "host says\n".to_owned(),
                sequence: 1,
                turn_id: String::new(),
                thread_id: String::new(),
            }),
            host_role: "host".to_owned(),
        });
        state.apply_event(RoomEvent::Crep {
            event: Box::new(CrepEvent::RoleOutputDelta {
                role: "backend".to_owned(),
                priors_hash: String::new(),
                text_delta: "backend says\n".to_owned(),
                sequence: 2,
                turn_id: String::new(),
                thread_id: String::new(),
            }),
            host_role: "host".to_owned(),
        });
        let text = state
            .scrollback
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(text.matches("@host").count(), 1);
        assert_eq!(text.matches("@backend").count(), 1);
    }

    #[test]
    fn notice_keeps_label_styled_in_scrollback() {
        let mut state = test_state();
        state.apply_event(RoomEvent::Notice {
            level: NoticeLevel::Warn,
            text: "approval pending".to_owned(),
        });
        let label_span = state
            .scrollback
            .iter()
            .flat_map(|line| line.spans.iter())
            .find(|span| span.content.as_ref() == "warn: ")
            .expect("warn label span present");
        assert_eq!(label_span.style.fg, Some(Color::Yellow));
        assert!(label_span.style.add_modifier.contains(Modifier::BOLD));
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
        // Spinner line carries the role avatar glyph before the braille
        // frame. Backend uses ◇ in the safe pack.
        assert!(text.contains("◇"));
    }

    #[test]
    fn spinner_line_prepends_role_glyph_and_keeps_status_text() {
        let snapshot = SpinnerSnapshot {
            role: "backend".to_owned(),
            frame: 0,
            started_at: Instant::now(),
            tools_seen: 0,
            current_state: Some("thinking".to_owned()),
            paint: SpinnerPaint::Painting,
        };
        let line = spinner_line(&snapshot, "host");
        let rendered: String = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<Vec<_>>()
            .join("");
        assert!(rendered.starts_with("◇ "));
        assert!(rendered.contains("@backend"));
        assert!(rendered.contains("thinking"));
    }

    #[test]
    fn spinner_line_role_glyph_and_role_token_share_color() {
        let snapshot = SpinnerSnapshot {
            role: "backend".to_owned(),
            frame: 0,
            started_at: Instant::now(),
            tools_seen: 0,
            current_state: None,
            paint: SpinnerPaint::Painting,
        };
        let line = spinner_line(&snapshot, "host");
        let glyph_fg = line.spans[0].style.fg;
        let token_idx = line
            .spans
            .iter()
            .position(|span| span.content.as_ref() == "@backend")
            .expect("@backend span present");
        let token_fg = line.spans[token_idx].style.fg;
        assert!(glyph_fg.is_some());
        assert_eq!(glyph_fg, token_fg);
    }

    #[test]
    fn work_cards_render_a_role_identity_header_per_card() {
        let mut state = test_state();
        state.apply_event(RoomEvent::WorkCard(sample_work_card()));
        let text = render_room_runtime_to_text(&state, 120, 30).expect("render");
        // The identity header line is `◇ @backend` (glyph + token).
        // The card body that follows still contains the title.
        assert!(text.contains("◇ @backend"));
        assert!(text.contains("Run validation"));
    }

    #[test]
    fn composer_candidate_menu_styles_role_mentions_with_role_color() {
        use crate::console_composer::ComposerCandidate;
        let candidate = ComposerCandidate {
            label: "@backend".to_owned(),
            description: String::new(),
            selected: false,
        };
        let spans = super::candidate_spans(&candidate, "host");
        assert_eq!(spans.len(), 1);
        let fg = spans[0].style.fg.expect("role mention has fg color");
        let expected = crate::tui_style::role_color("backend", "host");
        assert_eq!(fg, expected);
        assert!(spans[0].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn composer_candidate_menu_keeps_slash_commands_neutral() {
        use crate::console_composer::ComposerCandidate;
        let candidate = ComposerCandidate {
            label: "/help".to_owned(),
            description: String::new(),
            selected: false,
        };
        let spans = super::candidate_spans(&candidate, "host");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].style.fg, Some(Color::DarkGray));
        assert!(!spans[0].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn composer_candidate_selection_brackets_label() {
        use crate::console_composer::ComposerCandidate;
        let candidate = ComposerCandidate {
            label: "@backend".to_owned(),
            description: String::new(),
            selected: true,
        };
        let spans = super::candidate_spans(&candidate, "host");
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content.as_ref(), "[");
        assert_eq!(spans[1].content.as_ref(), "@backend");
        assert_eq!(spans[2].content.as_ref(), "]");
    }

    fn test_state() -> RoomRuntimeState {
        let team = vec![
            TeamMember {
                role: "host".to_owned(),
                engine: Engine::Cc,
            },
            TeamMember {
                role: "backend".to_owned(),
                engine: Engine::Cc,
            },
        ];
        RoomRuntimeState::new(
            PathBuf::from("/tmp/CoreRoom"),
            "host".to_owned(),
            vec!["host".to_owned(), "backend".to_owned()],
            team,
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
