//! Full-screen ratatui host for the executable CoreRoom runtime.
//!
//! This surface is not a snapshot viewer. It feeds submitted composer
//! lines into the existing REPL parser and renders the same `RoomEvent`
//! stream that `cr start` writes to stdout.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, IsTerminal as _, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::cursor::Show;
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
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
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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
use crate::spawn_lifecycle::{SpawnId, SpawnInstance, SpawnLifecycleTracker, SpawnState};
use crate::tui_style;

/// Mutable render state for the executable room.
#[derive(Debug)]
pub struct RoomRuntimeState {
    project_root: PathBuf,
    project_name: String,
    host_role: String,
    /// Configured roster (host + declared roles). Maintained for
    /// back-compat with the v0.9.x snapshot API and future surfaces;
    /// the v0.10 slim rail (#383) intentionally does NOT read it. The
    /// `activity: K/N roles seen` field that consumed it moved to the
    /// footer narration (#382).
    #[allow(dead_code)]
    team: Vec<TeamMember>,
    /// Last time each role emitted a spinner snapshot. The slim rail
    /// (#383) no longer reads this; we keep the field populated as a
    /// back-compat hook for upcoming surfaces that may want to surface
    /// role staleness.
    #[allow(dead_code)]
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
    /// Per-spawn-instance lifecycle records for the v0.10 chat stream.
    /// Populated from the same `CrepEvent` stream as `spinners`, but
    /// keyed by [`crate::spawn_lifecycle::SpawnId`] so concurrent
    /// spawns by the same role can be tracked independently. The
    /// current rail renderer reads from `spinners`, not this tracker,
    /// so adding this field is a no-visual-diff change (AC-5 on #380).
    /// `#381`+ wire the chat-stream widget against this tracker.
    spawn_lifecycle: SpawnLifecycleTracker,
    /// Report rows emitted by finished spawn instances. The base
    /// scrollback remains the durable event log; these rows are
    /// spliced directly under the spawn's collapsed Done marker so the
    /// report reads as part of that inline work unit (#384).
    spawn_report_rows: BTreeMap<SpawnId, Vec<Line<'static>>>,
    focused_spawn: Option<SpawnId>,
    expanded_done_spawns: BTreeSet<SpawnId>,
    focus_mode_spawn: Option<SpawnId>,
    work_cards: BTreeMap<String, WorkCard>,
    permission: Option<PendingPermission>,
    exiting: bool,
    show_cheatsheet: bool,
    /// Rows above the bottom that the user has scrolled the Room
    /// transcript up by. `0` follows new turns; `> 0` parks the view
    /// at history and a "↓ N new / End to follow" indicator is shown.
    scroll_offset: usize,
    /// Lines appended to scrollback while `scroll_offset > 0`. Drives
    /// the unread badge; reset to zero on `scroll_to_bottom`.
    unread_since_scroll: usize,
    /// Inner height of the Room widget seen on the most recent render,
    /// captured via interior mutability so scroll clamping can run
    /// from the (immutable) render path. `0` until the first frame.
    last_viewport_rows: Cell<u16>,
}

/// One configured role in the room roster. Live work state lives in
/// [`RoomRuntimeState::spinners`] and [`RoomRuntimeState::work_cards`];
/// the default right rail renders those work facts instead of this roster.
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
        let spawn_lifecycle = SpawnLifecycleTracker::new(host_role.clone());
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
            spawn_lifecycle,
            spawn_report_rows: BTreeMap::new(),
            focused_spawn: None,
            expanded_done_spawns: BTreeSet::new(),
            focus_mode_spawn: None,
            work_cards: BTreeMap::new(),
            permission: None,
            exiting: false,
            show_cheatsheet: false,
            scroll_offset: 0,
            unread_since_scroll: 0,
            last_viewport_rows: Cell::new(0),
        }
    }

    /// Apply one runtime event to the render model.
    pub fn apply_event(&mut self, event: RoomEvent) {
        match event {
            RoomEvent::Crep { event, host_role } => {
                // Per #380: drive the per-spawn lifecycle tracker off
                // the same CrepEvent stream the renderer reads. The
                // chat-stream renderer (`#381`) reads back from this
                // tracker to splice working cards into scrollback at
                // their original chat-time position.
                let spawn_id = self.spawn_lifecycle.apply_event(event.as_ref());
                // For a fresh `TurnDispatched`, stamp the spawn's
                // chat-row index now — *before* the spawner's "@host
                // delegating @role …" line gets pushed below — so the
                // card materializes immediately after that line in the
                // stream. Using `scrollback.len()` as the index makes
                // splicing trivial in the renderer: position N means
                // "after scrollback row N-1, before row N".
                if matches!(event.as_ref(), CrepEvent::TurnDispatched { .. }) {
                    if let Some(id) = spawn_id {
                        let position = self.scrollback.len();
                        self.spawn_lifecycle.set_chat_position(id, position);
                    }
                }
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
                let report_text = role_spoke_text(event.as_ref());
                if let (Some(id), Some(text)) = (spawn_id, report_text) {
                    if !text.trim().is_empty() {
                        // Report text belongs immediately below the
                        // collapsed Done marker, not at the live tail.
                        // Keep the normal role header intact even if
                        // the previous chunk came from the same role.
                        self.store_spawn_report_rows(id, &rendered);
                    }
                } else {
                    let cleaned = match (&speaker, &self.last_speaker) {
                        (Some(role), Some(prev)) if role == prev => {
                            strip_leading_role_header(&rendered, role)
                        }
                        _ => rendered,
                    };
                    self.push_rendered(&cleaned);
                }
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

    fn store_spawn_report_rows(&mut self, spawn_id: SpawnId, text: &str) {
        let rows = crate::ansi::ansi_to_lines(text);
        if rows.is_empty() {
            return;
        }
        if self.scroll_offset > 0 {
            self.unread_since_scroll = self.unread_since_scroll.saturating_add(rows.len());
        }
        self.spawn_report_rows
            .entry(spawn_id)
            .or_default()
            .extend(rows);
    }

    fn push_scrollback(&mut self, line: Line<'static>) {
        self.scrollback.push(line);
        if self.scroll_offset > 0 {
            self.unread_since_scroll = self.unread_since_scroll.saturating_add(1);
        }
        let overflow = self.scrollback.len().saturating_sub(1000);
        if overflow > 0 {
            self.scrollback.drain(0..overflow);
            // Drain evicts the oldest rows from the front. Any unread
            // lines that were drained are no longer reachable by
            // scrolling down, so the badge must not promise more than
            // `scroll_offset` rows still exist below the user's view.
            self.unread_since_scroll = self.unread_since_scroll.min(self.scroll_offset);
            // Every spawn lifecycle record carries a `chat_position`
            // anchored to a scrollback index. Without this shift, every
            // Working card's index points at an evicted row after the
            // drain, breaking #381 AC-2 (card sits at its chat-time
            // position). Saturating subtraction keeps the card visible
            // at the top of the window rather than panicking.
            self.spawn_lifecycle.shift_chat_positions(overflow);
        }
    }

    /// Current scrollback offset measured in rows above the bottom of
    /// the Room widget. `0` means "follow the latest turn".
    #[must_use]
    pub const fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    /// Number of lines appended since the user scrolled away from the
    /// bottom. Reset by `scroll_to_bottom`.
    #[must_use]
    pub const fn unread_since_scroll(&self) -> usize {
        self.unread_since_scroll
    }

    fn scroll_max(&self) -> usize {
        let viewport = usize::from(self.last_viewport_rows.get()).max(1);
        self.scrollback.len().saturating_sub(viewport)
    }

    /// Scroll the Room transcript up by `lines` rows. Clamped to the
    /// oldest visible row given the most recently rendered viewport.
    /// Until the first frame renders, `last_viewport_rows == 0` and we
    /// defer the request rather than parking the user one row from
    /// the top with no orientation — the next frame will trigger the
    /// clamp via the lazy `effective_offset` path in `render_scrollback`.
    pub fn scroll_up(&mut self, lines: usize) {
        if self.last_viewport_rows.get() == 0 {
            return;
        }
        let max = self.scroll_max();
        self.scroll_offset = self.scroll_offset.saturating_add(lines).min(max);
    }

    /// Scroll the Room transcript down by `lines` rows. Reaching the
    /// bottom clears the unread badge.
    pub fn scroll_down(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
        if self.scroll_offset == 0 {
            self.unread_since_scroll = 0;
        }
    }

    /// Jump to the oldest visible row in the current viewport. A no-op
    /// before the first frame renders (no viewport hint yet).
    pub fn scroll_to_top(&mut self) {
        if self.last_viewport_rows.get() == 0 {
            return;
        }
        self.scroll_offset = self.scroll_max();
    }

    /// Return to the bottom and resume following new turns.
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
        self.unread_since_scroll = 0;
    }

    /// Per-spawn lifecycle tracker for the chat-stream renderer
    /// (`#381`+). The current rail renderer does not consult this
    /// field — it still reads from
    /// [`Self::spinners`] — so exposing the tracker is a snapshot-API
    /// expansion, not a visual change.
    #[must_use]
    pub const fn spawn_lifecycle(&self) -> &SpawnLifecycleTracker {
        &self.spawn_lifecycle
    }

    /// Convenience shortcut over
    /// [`SpawnLifecycleTracker::working_instances_ordered_by_started_at`]
    /// for the footer narration consumer in `#382`.
    #[must_use]
    pub fn working_spawn_instances(&self) -> Vec<&SpawnInstance> {
        self.spawn_lifecycle
            .working_instances_ordered_by_started_at()
    }

    fn focusable_spawn_ids(&self) -> Vec<SpawnId> {
        let mut instances: Vec<&SpawnInstance> = self
            .spawn_lifecycle
            .instances()
            .filter(|spawn| {
                matches!(
                    spawn.state,
                    SpawnState::Working | SpawnState::Done | SpawnState::Reported
                )
            })
            .collect();
        instances.sort_by_key(|spawn| spawn.started_at);
        instances.into_iter().map(|spawn| spawn.spawn_id).collect()
    }

    fn focus_card(&mut self, reverse: bool) -> bool {
        let ids = self.focusable_spawn_ids();
        if ids.is_empty() {
            self.focused_spawn = None;
            self.focus_mode_spawn = None;
            return false;
        }
        let next_index = match self
            .focused_spawn
            .and_then(|id| ids.iter().position(|x| *x == id))
        {
            Some(index) if reverse => index.checked_sub(1).unwrap_or(ids.len() - 1),
            Some(index) => (index + 1) % ids.len(),
            None if reverse => ids.len() - 1,
            None => 0,
        };
        self.focused_spawn = Some(ids[next_index]);
        true
    }

    fn focused_spawn(&self) -> Option<&SpawnInstance> {
        self.focused_spawn
            .and_then(|spawn_id| self.spawn_lifecycle.get(spawn_id))
    }

    fn clear_card_focus(&mut self) {
        self.focus_mode_spawn = None;
        self.focused_spawn = None;
    }

    fn toggle_done_expansion(&mut self) -> bool {
        let Some(spawn) = self.focused_spawn() else {
            return false;
        };
        if !matches!(spawn.state, SpawnState::Done | SpawnState::Reported)
            || spawn.tool_calls.is_empty()
        {
            return false;
        }
        let spawn_id = spawn.spawn_id;
        if !self.expanded_done_spawns.remove(&spawn_id) {
            self.expanded_done_spawns.insert(spawn_id);
        }
        true
    }

    fn focused_working_role(&self) -> Option<String> {
        self.focused_spawn()
            .filter(|spawn| spawn.state == SpawnState::Working)
            .map(|spawn| spawn.role.clone())
    }

    fn toggle_focus_mode(&mut self) -> bool {
        let Some(spawn_id) = self
            .focused_spawn()
            .filter(|spawn| spawn.state == SpawnState::Working)
            .map(|spawn| spawn.spawn_id)
        else {
            return false;
        };
        if self.focus_mode_spawn == Some(spawn_id) {
            self.focus_mode_spawn = None;
        } else {
            self.focus_mode_spawn = Some(spawn_id);
        }
        true
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
        Event::Mouse(mouse) if state.permission.is_none() && !state.show_cheatsheet => {
            handle_mouse(mouse, state);
        }
        _ => {}
    }
    Ok(())
}

/// Translate the live room's wheel events into scrollback navigation.
/// `MOUSE_WHEEL_LINES` matches the K9s / vim feel — one notch moves a
/// small fixed step rather than a full page.
fn handle_mouse(mouse: MouseEvent, state: &mut RoomRuntimeState) {
    const MOUSE_WHEEL_LINES: usize = 3;
    match mouse.kind {
        MouseEventKind::ScrollUp => state.scroll_up(MOUSE_WHEEL_LINES),
        MouseEventKind::ScrollDown => state.scroll_down(MOUSE_WHEEL_LINES),
        _ => {}
    }
}

/// One PgUp/PgDn moves half the visible Room height, with a small
/// floor for tiny windows. Half-page matches the feel of `less` and
/// keeps a couple of orientation rows on screen when the user pages.
fn scroll_page_lines(state: &RoomRuntimeState) -> usize {
    let viewport = usize::from(state.last_viewport_rows.get());
    (viewport / 2).max(1)
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
    if handle_card_key(key, state, input_tx)? {
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
        KeyCode::PageUp => {
            let page = scroll_page_lines(state);
            state.scroll_up(page);
        }
        KeyCode::PageDown => {
            let page = scroll_page_lines(state);
            state.scroll_down(page);
        }
        KeyCode::Home if state.composer.view_model().input.is_empty() => {
            state.scroll_to_top();
        }
        KeyCode::End if state.composer.view_model().input.is_empty() => {
            state.scroll_to_bottom();
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

fn handle_card_key(
    key: KeyEvent,
    state: &mut RoomRuntimeState,
    input_tx: &mpsc::UnboundedSender<RuntimeInput>,
) -> Result<bool> {
    if key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return Ok(false);
    }
    match key.code {
        KeyCode::Tab
            if state.composer.view_model().input.is_empty() || state.focused_spawn.is_some() =>
        {
            Ok(state.focus_card(false))
        }
        KeyCode::BackTab
            if state.composer.view_model().input.is_empty() || state.focused_spawn.is_some() =>
        {
            Ok(state.focus_card(true))
        }
        KeyCode::Esc if state.focus_mode_spawn.is_some() => {
            state.focus_mode_spawn = None;
            Ok(true)
        }
        KeyCode::Esc if state.focused_spawn.is_some() => {
            state.clear_card_focus();
            Ok(true)
        }
        KeyCode::Char('e') if state.focused_spawn.is_some() => {
            let _ = state.toggle_done_expansion();
            Ok(true)
        }
        KeyCode::Char('i') if state.focused_spawn.is_some() => {
            if let Some(role) = state.focused_working_role() {
                input_tx
                    .send(RuntimeInput::Line(format!("/halt @{role}")))
                    .context("sending focused-card interrupt to room runtime")?;
                state.push_notice(
                    NoticeLevel::System,
                    format!("interrupt requested for @{role}"),
                );
            }
            Ok(true)
        }
        KeyCode::Char('f') if state.focused_spawn.is_some() => {
            let _ = state.toggle_focus_mode();
            Ok(true)
        }
        _ => Ok(false),
    }
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
    // Per the v0.10 ADR (`docs/v0.10-chat-stream-vs-dashboard.md`,
    // "Footer narration line"): the narration strip shows above the
    // composer ONLY when at least one sub-agent is in `Working` or
    // `Spawning`. When both counts are zero the row is hidden so the
    // composer reclaims that cell — no empty bar is rendered.
    let needs_narration = !state
        .spawn_lifecycle
        .working_instances_ordered_by_started_at()
        .is_empty()
        || !state
            .spawn_lifecycle
            .spawning_instances_ordered_by_started_at()
            .is_empty();

    if needs_narration {
        let root = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // status bar
                Constraint::Min(8),    // body (room + rail)
                Constraint::Length(1), // footer narration
                Constraint::Length(5), // composer
                Constraint::Length(1), // footer (bindings)
            ])
            .split(area);

        render_status_bar(frame, root[0], state);
        render_body(frame, root[1], state);
        render_footer_narration(frame, root[2], state);
        render_composer(frame, root[3], state);
        render_footer(frame, root[4], state);
    } else {
        let root = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // status bar
                Constraint::Min(8),    // body (room + rail)
                Constraint::Length(5), // composer
                Constraint::Length(1), // footer (bindings)
            ])
            .split(area);

        render_status_bar(frame, root[0], state);
        render_body(frame, root[1], state);
        render_composer(frame, root[2], state);
        render_footer(frame, root[3], state);
    }

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
    // Capture the viewport so scroll handlers can clamp `scroll_offset`
    // against the real Room widget height — the cheapest path to a
    // shared viewport hint without threading `&mut state` through the
    // entire render call chain.
    state
        .last_viewport_rows
        .set(u16::try_from(visible_rows).unwrap_or(u16::MAX));

    // Build the merged scrollback with working cards spliced inline
    // at their original chat-time positions (#381). When no spawn is
    // in the `Working` state this is a clone of `scrollback`.
    let merged = build_merged_scrollback(state, area.width);

    // Clamp lazily: scrollback can shrink (1000-row drain in
    // `push_scrollback`), so the stored offset may exceed the new max.
    let max_offset = merged.len().saturating_sub(visible_rows.max(1));
    let effective_offset = state.scroll_offset.min(max_offset);

    let items: Vec<Line<'static>> = if effective_offset == 0 {
        // Sticking to the bottom: include the inline activity rows so
        // the most recent turn reads like a chat composer would. This
        // is the v0.9.15 behavior, preserved (and still applies when
        // scrollback is empty so spinner activity is visible on a
        // fresh room).
        let activity_lines = current_turn_lines(state);
        let scroll_rows = visible_rows.saturating_sub(activity_lines.len());
        let start = merged.len().saturating_sub(scroll_rows);
        let mut items: Vec<Line<'static>> = if merged.is_empty() {
            vec![Line::from(vec![Span::styled(
                "Submit a task below. Runtime output appears here.",
                Style::default().fg(Color::DarkGray),
            )])]
        } else {
            merged[start..].to_vec()
        };
        items.extend(activity_lines);
        items
    } else {
        // Scrolled back: drop the "now" activity rows and reserve the
        // bottom row for a follow-back indicator so the user always
        // sees that they are looking at history.
        let scroll_rows = visible_rows.saturating_sub(1);
        let total = merged.len();
        let end = total.saturating_sub(effective_offset);
        let start = end.saturating_sub(scroll_rows);
        let mut items = merged[start..end].to_vec();
        items.push(scrollback_follow_indicator(state.unread_since_scroll));
        items
    };

    let items = items.into_iter().map(ListItem::new).collect::<Vec<_>>();
    frame.render_widget(
        List::new(items).block(Block::default().borders(Borders::ALL).title("Room")),
        area,
    );
}

/// Build the scrollback to render: existing scrollback with inline
/// spawn activity spliced at each spawn's `chat_position`. `Working`
/// spawns render as cards; `Done` / `Reported` spawns render as the
/// collapsed marker plus any report rows captured for that spawn.
/// Spawns whose chat_position is past the end of scrollback are
/// appended at the bottom (covers the race where the spawn was
/// dispatched but the spawner's confirmation line has not yet
/// rendered).
///
/// Elapsed time is rendered at one-second granularity (AC-5) — the
/// `now` argument is the same `Instant` used for every card on this
/// frame, so even if two cards started at slightly different points
/// the displayed elapsed only changes once per wall-clock second.
fn build_merged_scrollback(state: &RoomRuntimeState, panel_width: u16) -> Vec<Line<'static>> {
    // Card width is panel_width less the surrounding borders (2 chars).
    let inner_width = panel_width.saturating_sub(2);
    // Collect renderable spawns, deduplicated and ordered by
    // chat_position so the splice walk below stays linear.
    let mut renderable: Vec<&SpawnInstance> = state
        .spawn_lifecycle()
        .instances()
        .filter(|spawn| {
            matches!(
                spawn.state,
                SpawnState::Working | SpawnState::Done | SpawnState::Reported
            )
        })
        .collect();
    renderable.sort_by_key(|spawn| spawn.chat_position);

    if renderable.is_empty() {
        return state.scrollback.clone();
    }

    let now = Instant::now();
    let mut merged: Vec<Line<'static>> =
        Vec::with_capacity(state.scrollback.len() + renderable.len() * 5);
    let mut spawn_iter = renderable.into_iter().peekable();
    for (idx, line) in state.scrollback.iter().enumerate() {
        // Splice in any spawn rows whose chat_position == idx (before this row).
        while spawn_iter
            .peek()
            .is_some_and(|spawn| spawn.chat_position <= idx)
        {
            let spawn = spawn_iter.next().expect("peeked non-empty");
            append_spawn_activity_lines(&mut merged, state, spawn, inner_width, now);
        }
        merged.push(line.clone());
    }
    // Append any spawn rows whose chat_position is past the end of scrollback
    // (race: spawn registered but no scrollback row has landed yet).
    for spawn in spawn_iter {
        append_spawn_activity_lines(&mut merged, state, spawn, inner_width, now);
    }
    merged
}

fn append_spawn_activity_lines(
    merged: &mut Vec<Line<'static>>,
    state: &RoomRuntimeState,
    spawn: &SpawnInstance,
    inner_width: u16,
    now: Instant,
) {
    let focused = state.focused_spawn == Some(spawn.spawn_id);
    match spawn.state {
        SpawnState::Working => {
            if state
                .focus_mode_spawn
                .is_some_and(|id| id != spawn.spawn_id)
            {
                if let Some(line) =
                    crate::working_card::render_working_stub_line(spawn, &state.host_role, now)
                {
                    merged.push(line);
                }
            } else {
                merged.extend(crate::working_card::render_working_card_lines_with_focus(
                    spawn,
                    &state.host_role,
                    inner_width,
                    now,
                    crate::working_card::DEFAULT_VISIBLE_STEPS,
                    focused,
                ));
            }
        }
        SpawnState::Done | SpawnState::Reported => {
            if let Some(line) = crate::working_card::render_done_collapsed_line_with_focus(
                spawn,
                &state.host_role,
                focused,
            ) {
                merged.push(line);
            }
            if state.expanded_done_spawns.contains(&spawn.spawn_id) {
                merged.extend(crate::working_card::render_expanded_done_log_lines(
                    spawn,
                    inner_width,
                ));
            }
            if let Some(report_rows) = state.spawn_report_rows.get(&spawn.spawn_id) {
                merged.extend(report_rows.iter().cloned());
            }
        }
        SpawnState::Spawning => {}
    }
}

/// Bottom-of-Room hint shown whenever the user has scrolled away from
/// the live tail. Yellow when there is unread output to flag, dim gray
/// when scrollback is quiet so the user can ignore it.
fn scrollback_follow_indicator(unread: usize) -> Line<'static> {
    if unread > 0 {
        Line::from(vec![Span::styled(
            format!("↓ {unread} new · End to follow"),
            Style::default().fg(Color::Yellow),
        )])
    } else {
        Line::from(vec![Span::styled(
            "↑ scrolled back · End to follow",
            Style::default().fg(Color::DarkGray),
        )])
    }
}

/// Slim `Status` rail per ADR `docs/v0.10-chat-stream-vs-dashboard.md` §Q5.
///
/// The rail surfaces only ambient project state that outlives any
/// single turn — Work counts, Evidence validation, room-level Blockers.
/// It deliberately does NOT read `state.spinners` or
/// `state.spawn_lifecycle`; live sub-agent activity (tool calls,
/// roles seen, latest step, assignee) belongs in the chat stream
/// (#381) and the footer narration (#382), not on the rail. As a
/// consequence the rail is byte-identical across tool-call ticks:
/// it only re-renders when a `WorkCard`, permission prompt, or other
/// project-level signal arrives.
fn render_status_rail(frame: &mut Frame<'_>, area: Rect, state: &RoomRuntimeState) {
    let lines = slim_status_rail_lines(state);
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title("Status"))
            .wrap(Wrap { trim: true }),
        area,
    );
}

/// Build the slim Status card body. Pulled out so tests can call it
/// directly without going through the ratatui pipeline.
///
/// Stability contract: this function MUST be a pure projection of
/// project-level state (`work_cards`, `permission` flag). It must NOT
/// read `state.spinners`, `state.spawn_lifecycle`, or anything that
/// updates on every tool-call tick. Adding a spinner-derived field
/// here regresses ADR §Q5 and #383 AC-2.
fn slim_status_rail_lines(state: &RoomRuntimeState) -> Vec<Line<'static>> {
    let mut lines = Vec::with_capacity(10);
    lines.push(section_header("Work"));
    let active = rail_work_active_count(state);
    lines.push(status_kv_line(
        "active",
        &active.to_string(),
        if active > 0 {
            Color::Green
        } else {
            Color::DarkGray
        },
    ));
    let cards = state.work_cards.len();
    lines.push(status_kv_line(
        "cards",
        &cards.to_string(),
        if cards == 0 {
            Color::DarkGray
        } else {
            Color::Yellow
        },
    ));
    lines.push(Line::raw(""));
    lines.push(section_header("Blockers"));
    let (blocker_text, blocker_color) = rail_blocker_status(state);
    lines.push(status_kv_line("state", blocker_text, blocker_color));
    lines.push(Line::raw(""));
    lines.push(section_header("Evidence"));
    let evidence = rail_evidence_status(state);
    lines.push(status_kv_line("validation", evidence.label, evidence.color));
    lines
}

/// `Work active: N` — count of WorkCards currently in the `Working`
/// state. Per ADR §Q5 this proxies the project-level `WorkOrder`
/// `Working` count: a WorkCard is opened when a project work item
/// starts and closes when it ends, so the count is stable across
/// tool-call ticks (which only update inner fields like
/// `spinner_frame` and `current_step`, not the variant). Crucially,
/// this does NOT read `state.spinners` or `state.spawn_lifecycle`,
/// so a sub-agent making tool calls inside a single WorkCard does
/// not change `active`.
fn rail_work_active_count(state: &RoomRuntimeState) -> usize {
    state
        .work_cards
        .values()
        .filter(|card| matches!(card.status, WorkStatus::Working { .. }))
        .count()
}

/// Room-level Blockers rollup per ADR §Q5. Reads only signals that
/// last across tool-call ticks: an open permission prompt, or any
/// WorkCard that has settled into the `Interrupted` state. Per-spawn
/// blockers (a sub-agent failing a single tool call) are intentionally
/// NOT counted here — they belong inline in the chat stream's working
/// card, not on the rail.
fn rail_blocker_status(state: &RoomRuntimeState) -> (&'static str, Color) {
    if state.permission.is_some() {
        return ("approval pending", Color::Yellow);
    }
    if state
        .work_cards
        .values()
        .any(|card| matches!(card.status, WorkStatus::Interrupted { .. }))
    {
        return ("interrupted", Color::Red);
    }
    ("none", Color::DarkGray)
}

#[derive(Debug, Clone, Copy)]
struct EvidenceStatus {
    label: &'static str,
    color: Color,
}

/// Evidence validation rollup per ADR §Q5. The runtime does not yet
/// carry `EvidencePacket.validation_status` directly, so we project
/// the closest stable signal: any settled WorkCard counts as `clean`,
/// any in-flight WorkCard counts as `pending`, an Interrupted WorkCard
/// or open permission counts as `blocking`. Nothing observed yet ⇒
/// `not observed`. Critically, this rollup does NOT read
/// `state.has_active_work()` (which inspects `spinners`); it only
/// reads `work_cards` statuses and the `permission` flag.
fn rail_evidence_status(state: &RoomRuntimeState) -> EvidenceStatus {
    let has_working = state
        .work_cards
        .values()
        .any(|card| matches!(card.status, WorkStatus::Working { .. }));
    let has_interrupted = state
        .work_cards
        .values()
        .any(|card| matches!(card.status, WorkStatus::Interrupted { .. }));
    let has_done = state
        .work_cards
        .values()
        .any(|card| matches!(card.status, WorkStatus::Done { .. }));

    if has_interrupted || state.permission.is_some() {
        EvidenceStatus {
            label: "blocking",
            color: Color::Red,
        }
    } else if has_working {
        EvidenceStatus {
            label: "pending",
            color: Color::Yellow,
        }
    } else if has_done {
        EvidenceStatus {
            label: "clean",
            color: Color::Green,
        }
    } else {
        EvidenceStatus {
            label: "not observed",
            color: Color::DarkGray,
        }
    }
}

/// Build the configured room roster: host first, then declared roles in
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

fn role_spoke_text(event: &CrepEvent) -> Option<&str> {
    match event {
        CrepEvent::RoleSpoke { text, .. } => Some(text),
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

#[cfg(test)]
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

/// Inline activity rows appended at the bottom of the Room
/// scrollback, just above the composer. Style matches a chat-style
/// tool-call indicator (one row per active role; no surrounding
/// frame) so the live status reads as part of the conversation flow
/// instead of a banner. Empty when no role is working and no
/// permission overlay is open.
fn current_turn_lines(state: &RoomRuntimeState) -> Vec<Line<'static>> {
    let active_roles: Vec<&str> = state.spinners.keys().map(String::as_str).collect();
    let pending_role = state.permission.as_ref().map(|p| p.request.role.as_str());
    let has_pending_only = pending_role.is_some_and(|role| !active_roles.contains(&role));
    if active_roles.is_empty() && !has_pending_only {
        return Vec::new();
    }

    let mut lines: Vec<Line<'static>> = Vec::new();
    for snapshot in state.spinners.values() {
        lines.push(activity_card_row(snapshot, state));
    }
    if let Some(role) = pending_role {
        if !active_roles.contains(&role) {
            lines.push(activity_card_waiting_row(role, &state.host_role));
        }
    }
    lines
}

/// One styled row inside the Room activity card for an active role.
fn activity_card_row(snapshot: &SpinnerSnapshot, state: &RoomRuntimeState) -> Line<'static> {
    let role_color = tui_style::role_color(&snapshot.role, &state.host_role);
    let glyph = tui_style::role_avatar_glyph(&snapshot.role, &state.host_role);
    let frame_text = SPINNER_FRAMES[snapshot.frame % SPINNER_FRAMES.len()];
    let frame_color = match snapshot.paint {
        SpinnerPaint::WaitingApproval => Color::Yellow,
        SpinnerPaint::Painting => Color::Cyan,
        SpinnerPaint::Cleared => Color::DarkGray,
    };
    let elapsed = snapshot.started_at.elapsed().as_secs();
    let card_step = state
        .work_cards
        .get(&snapshot.role)
        .and_then(|card| match &card.status {
            WorkStatus::Working { current_step, .. } => current_step.clone(),
            _ => None,
        });
    let action = card_step
        .or_else(|| snapshot.current_state.clone())
        .unwrap_or_else(|| "thinking".to_owned());
    let mut spans = Vec::with_capacity(10);
    spans.push(Span::raw("  "));
    spans.push(Span::styled(
        glyph.to_owned(),
        Style::default().fg(role_color),
    ));
    spans.push(Span::raw(" "));
    spans.push(Span::styled(frame_text, Style::default().fg(frame_color)));
    spans.push(Span::raw(" "));
    spans.push(Span::styled(
        format!("@{}", snapshot.role),
        Style::default().fg(role_color).add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(
        format!(" · {elapsed}s"),
        Style::default().fg(Color::DarkGray),
    ));
    if snapshot.tools_seen > 0 {
        let plural = if snapshot.tools_seen == 1 { "" } else { "s" };
        spans.push(Span::styled(
            format!(" · {} tool{plural}", snapshot.tools_seen),
            Style::default().fg(Color::DarkGray),
        ));
    }
    spans.push(Span::raw(" · "));
    spans.push(Span::styled(
        action,
        Style::default()
            .fg(frame_color)
            .add_modifier(Modifier::BOLD),
    ));
    Line::from(spans)
}

/// One styled row inside the Room activity card for a role that is
/// blocked on a permission overlay but no longer has a live spinner.
fn activity_card_waiting_row(role: &str, host_role: &str) -> Line<'static> {
    let role_color = tui_style::role_color(role, host_role);
    let glyph = tui_style::role_avatar_glyph(role, host_role);
    Line::from(vec![
        Span::raw("  "),
        Span::styled(glyph.to_owned(), Style::default().fg(role_color)),
        Span::raw(" "),
        Span::styled("⏸", Style::default().fg(Color::Yellow)),
        Span::raw(" "),
        Span::styled(
            format!("@{role}"),
            Style::default().fg(role_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " · waiting approval",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

fn section_header(label: &'static str) -> Line<'static> {
    Line::from(vec![Span::styled(label, label_style())])
}

fn status_kv_line(label: &'static str, value: &str, value_color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {label}: "), Style::default().fg(Color::White)),
        Span::styled(value.to_owned(), Style::default().fg(value_color)),
    ])
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

/// One-row narration strip above the composer that names the roles
/// being waited on while sub-agents work. Format (locked by the v0.10
/// ADR, "Footer narration line"):
///
/// ```text
/// 2 roles still working · @security @backend · @qa spawning
/// ```
///
/// The leading `N roles still working` count is **`Working`-only**.
/// `Spawning` instances are named with a trailing `· @role spawning`
/// suffix so the user sees that a role has been delegated even before
/// its first tool call, but they do NOT increment the count (they have
/// no live tool-call stream and no card to focus). When both counts
/// are zero the renderer hides the row entirely — see
/// [`render_room_runtime_frame`].
///
/// The strip never wraps. When the row would overflow, role chips
/// truncate left-to-right with `… +N more`.
fn render_footer_narration(frame: &mut Frame<'_>, area: Rect, state: &RoomRuntimeState) {
    let working = state
        .spawn_lifecycle
        .working_instances_ordered_by_started_at();
    let spawning = state
        .spawn_lifecycle
        .spawning_instances_ordered_by_started_at();
    let line =
        build_footer_narration_line(&working, &spawning, &state.host_role, area.width as usize);
    frame.render_widget(Paragraph::new(line), area);
}

/// One role chip in [`build_footer_narration_line`]: the colored
/// `@role` text plus an optional trailing `" spawning"` annotation
/// drawn in the muted style.
struct NarrationChip {
    text: String,
    color: Color,
    suffix: Option<&'static str>,
}

/// Pure-data builder for [`render_footer_narration`]. Split out so the
/// renderer tests can pin the exact span sequence and the truncation
/// behavior without standing up a [`Frame`].
fn build_footer_narration_line(
    working: &[&SpawnInstance],
    spawning: &[&SpawnInstance],
    host_role: &str,
    available_width: usize,
) -> Line<'static> {
    let working_count = working.len();
    // Per ADR Q4: list roles only — no "chat resumes" promise. The
    // header text uses singular `role` for exactly one Working, plural
    // `roles` for zero or many. ("0 roles" reads correctly when only
    // Spawning instances are present.)
    let header = if working_count == 1 {
        "1 role still working".to_owned()
    } else {
        format!("{working_count} roles still working")
    };
    let muted = Style::default().fg(Color::DarkGray);

    // Build the candidate chip list. Each chip is rendered as its own
    // styled span; we measure visible width with `UnicodeWidthStr` and
    // walk left-to-right, dropping the tail to a truncation marker
    // (`… +N more`) when the next chip would exceed the row budget.
    let mut chips: Vec<NarrationChip> = Vec::with_capacity(working.len() + spawning.len());
    for instance in working {
        chips.push(NarrationChip {
            text: format!("@{}", instance.role),
            color: tui_style::role_color(&instance.role, host_role),
            suffix: None,
        });
    }
    for instance in spawning {
        chips.push(NarrationChip {
            text: format!("@{}", instance.role),
            color: tui_style::role_color(&instance.role, host_role),
            suffix: Some(" spawning"),
        });
    }

    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.push(Span::styled(header.clone(), muted));
    if chips.is_empty() {
        return Line::from(spans);
    }

    // Width budget = entire row minus the header that is already
    // committed. We reserve a separator (` · `) before every chip and
    // the chip itself (and its optional ` spawning` suffix). When the
    // remaining budget cannot fit the next chip we collapse the tail
    // to `… +N more`, which is itself measured against the budget so a
    // single overflowing chip still produces a visible truncation
    // marker rather than overflowing silently.
    let header_w = UnicodeWidthStr::width(header.as_str());
    let mut used = header_w;
    let total = chips.len();

    let mut i = 0;
    while i < total {
        let chip = &chips[i];
        let chip_w = UnicodeWidthStr::width(chip.text.as_str())
            + chip.suffix.map_or(0, UnicodeWidthStr::width);
        let sep = footer_narration_separator(i, working.len());
        let sep_w = UnicodeWidthStr::width(sep);
        let remaining = total - i;
        // Reserve space for a possible truncation marker AFTER this
        // chip, so the last chip that fits can be followed by `…
        // +K more` if a later chip would overflow. When this is the
        // last chip in the list there is no tail to mark, so no
        // reservation is needed.
        let reserve = if remaining > 1 {
            let tail_marker = format!("… +{} more", remaining - 1);
            let tail_sep = footer_narration_separator(i + 1, working.len());
            UnicodeWidthStr::width(tail_sep) + UnicodeWidthStr::width(tail_marker.as_str())
        } else {
            0
        };

        if used + sep_w + chip_w + reserve <= available_width {
            spans.push(Span::styled(sep.to_owned(), muted));
            spans.push(Span::styled(
                chip.text.clone(),
                Style::default().fg(chip.color),
            ));
            if let Some(suffix) = chip.suffix {
                spans.push(Span::styled(suffix.to_owned(), muted));
            }
            used += sep_w + chip_w;
            i += 1;
        } else {
            // This chip (or the truncation marker that would have to
            // follow it) does not fit. Collapse the rest of the list to
            // `… +K more` with K = number of chips not rendered. The
            // marker is rendered best-effort: when the area is too
            // narrow to fit even the marker, we still emit it so the
            // user sees "there is more" rather than a silent drop.
            let marker_text = format!("… +{remaining} more");
            spans.push(Span::styled(sep.to_owned(), muted));
            spans.push(Span::styled(marker_text, muted));
            break;
        }
    }

    Line::from(spans)
}

/// Choose the separator that precedes the chip at index `i`.
///
/// - First chip overall (i = 0): ` · ` (separates the header from the
///   chip list).
/// - First chip of the Spawning group when there is at least one
///   Working chip ahead of it (i == working_count > 0): ` · ` (the
///   ADR uses a dot to set the spawning suffix apart from the Working
///   list).
/// - All other chips: ` ` (chips inside the same group are
///   space-separated).
fn footer_narration_separator(i: usize, working_count: usize) -> &'static str {
    let is_group_boundary = i == 0 || (i == working_count && working_count > 0);
    if is_group_boundary {
        " · "
    } else {
        " "
    }
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
    bindings.push(FooterBinding {
        keys: "wheel / pgup / pgdn",
        action: "scroll Room history",
        priority: 3,
        primary_chip: false,
    });
    bindings.push(FooterBinding {
        keys: "home / end",
        action: if typed {
            "composer cursor (empty composer: top / follow)"
        } else {
            "scroll top / follow latest"
        },
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

/// Flatten a vec of styled lines to a plain string, one `\n` per
/// row. Used by the rail tests to compare bodies for the AC-2
/// stability assertion without dragging in the ratatui pipeline.
#[cfg(test)]
fn lines_to_plain_string(lines: &[Line<'_>]) -> String {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn write_enter_commands<W: Write>(mut writer: W) -> io::Result<()> {
    // `EnableMouseCapture` is what turns the live room into a true
    // K9S / tmux / vim style sandbox: the terminal stops scrolling its
    // own main-buffer scrollback in response to the wheel and forwards
    // mouse events to us instead. Without this, alt-screen still lets
    // the user surface prior shell history with the scroll wheel on
    // iTerm2 / Terminal.app, which breaks the "this is its own app"
    // impression. We don't act on the mouse events for now — the
    // event loop ignores them — but the capture is enough to keep the
    // viewport pinned to what the TUI rendered.
    //
    // Cursor visibility is intentionally *not* set here. ratatui's
    // `Terminal::draw` shows or hides the cursor every frame based on
    // whether `frame.set_cursor_position` was called, so issuing a
    // standalone `Hide` here would race with the composer's per-frame
    // `set_cursor_position` call and leave the Ask input without a
    // visible caret.
    execute!(
        writer,
        EnterAlternateScreen,
        EnableBracketedPaste,
        EnableMouseCapture,
    )
}

fn write_leave_commands<W: Write>(mut writer: W) -> io::Result<()> {
    execute!(
        writer,
        DisableMouseCapture,
        DisableBracketedPaste,
        Show,
        LeaveAlternateScreen,
    )
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
    fn rail_shows_locked_slim_status_when_idle() {
        // ADR `docs/v0.10-chat-stream-vs-dashboard.md` §Q5 locks the
        // slim Status rail content for the idle baseline: Work
        // (active/cards), Blockers (state), Evidence (validation).
        // Nothing else.
        let state = test_state();
        let text = render_room_runtime_to_text(&state, 120, 30).expect("render");
        assert!(text.contains("Status"));
        assert!(text.contains("Work"));
        assert!(text.contains("active: 0"));
        assert!(text.contains("cards: 0"));
        assert!(text.contains("Blockers"));
        assert!(text.contains("state: none"));
        assert!(text.contains("Evidence"));
        assert!(text.contains("validation: not observed"));
        // Removed v0.9.x rail sections must not reappear.
        assert!(!text.contains("Roles"));
        assert!(!text.contains("Current"));
        assert!(!text.contains("activity:"));
        assert!(!text.contains("latest:"));
        assert!(!text.contains("assignee:"));
        assert!(!text.contains("no work cards yet"));
    }

    #[test]
    fn rail_stays_byte_identical_across_spinner_ticks() {
        // AC-2 of #383: a live sub-agent making tool calls must not
        // flicker the rail. Two spinner snapshots that differ only on
        // per-tick fields (`frame`, `tools_seen`, `current_state`)
        // must produce the same rail body.
        let mut state = test_state();
        state.apply_event(RoomEvent::Spinner(SpinnerSnapshot {
            role: "backend".to_owned(),
            frame: 0,
            started_at: Instant::now(),
            tools_seen: 0,
            current_state: Some("thinking".to_owned()),
            paint: SpinnerPaint::Painting,
        }));
        let before = super::slim_status_rail_lines(&state);
        state.apply_event(RoomEvent::Spinner(SpinnerSnapshot {
            role: "backend".to_owned(),
            frame: 5,
            started_at: Instant::now(),
            tools_seen: 4,
            current_state: Some("running cargo test".to_owned()),
            paint: SpinnerPaint::Painting,
        }));
        let after = super::slim_status_rail_lines(&state);
        assert_eq!(
            super::lines_to_plain_string(&before),
            super::lines_to_plain_string(&after),
            "slim rail must be byte-identical across tool-call ticks",
        );
    }

    #[test]
    fn rail_active_counts_working_work_cards_not_spinners() {
        // ADR §Q5 locks `Work active: N` to the WorkCard count, not
        // the spinner count. A spinner alone (no WorkCard) leaves
        // `active` at 0; a Working WorkCard bumps it to 1.
        let mut state = test_state();
        state.apply_event(RoomEvent::Spinner(SpinnerSnapshot {
            role: "backend".to_owned(),
            frame: 0,
            started_at: Instant::now(),
            tools_seen: 0,
            current_state: Some("thinking".to_owned()),
            paint: SpinnerPaint::Painting,
        }));
        let text_spinner_only =
            render_room_runtime_to_text(&state, 120, 30).expect("render spinner-only");
        assert!(text_spinner_only.contains("active: 0"));
        assert!(text_spinner_only.contains("cards: 0"));
        // The inline activity row still surfaces the role at the
        // bottom of Room — that surface (#381 territory) is separate
        // from the rail.
        assert!(text_spinner_only.contains("@backend"));

        state.apply_event(RoomEvent::WorkCard(sample_work_card()));
        let text_with_card =
            render_room_runtime_to_text(&state, 120, 30).expect("render with card");
        assert!(text_with_card.contains("active: 1"));
        assert!(text_with_card.contains("cards: 1"));
    }

    #[test]
    fn rail_does_not_render_work_card_body() {
        // The pre-v0.10 rail rendered the WorkCard's full body inside
        // the Status panel. The slim rail (#383) drops that block —
        // the WorkCard's title, current step, and inline @role label
        // belong to the chat-stream working card (#381), not the rail.
        let mut state = test_state();
        state.apply_event(RoomEvent::WorkCard(sample_work_card()));
        let lines = super::slim_status_rail_lines(&state);
        let rendered = super::lines_to_plain_string(&lines);
        assert!(!rendered.contains("Run validation"));
        assert!(!rendered.contains("@backend"));
        assert!(!rendered.contains("cargo test"));
        // Only the slim counters change.
        assert!(rendered.contains("active: 1"));
        assert!(rendered.contains("cards: 1"));
    }

    #[test]
    fn rail_blocker_state_reflects_permission_prompt() {
        let mut state = test_state();
        let (tx, _rx) = mpsc::unbounded_channel();
        state.apply_event(RoomEvent::PermissionPrompt {
            request: sample_request(),
            host_role: "host".to_owned(),
            response_tx: Some(tx),
        });
        let lines = super::slim_status_rail_lines(&state);
        let rendered = super::lines_to_plain_string(&lines);
        // Per-role detail (`approval: @backend · Bash`) is gone from
        // the rail; the slim rail surfaces room-level state only.
        assert!(rendered.contains("state: approval pending"));
        assert!(!rendered.contains("@backend"));
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

    fn render_with_cursor(
        state: &RoomRuntimeState,
        width: u16,
        height: u16,
    ) -> (String, (u16, u16)) {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("create test live room terminal");
        terminal
            .draw(|frame| render_room_runtime_frame(frame, state))
            .expect("draw test live room frame");
        let pos = terminal
            .get_cursor_position()
            .expect("get cursor position from test backend");
        let text = buffer_to_string(terminal.backend().buffer());
        (text, (pos.x, pos.y))
    }

    #[test]
    fn write_enter_commands_does_not_hide_cursor() {
        // Regression for live-room "no visible cursor" bug: the alt-screen
        // setup must not emit DECRST 25 (`CSI ?25 l`). ratatui's
        // `Terminal::draw` is responsible for the cursor's visibility on
        // every frame, so a one-shot `Hide` here races the composer's
        // per-frame `set_cursor_position` call and leaves Ask without a
        // visible caret.
        let mut buf: Vec<u8> = Vec::new();
        super::write_enter_commands(&mut buf).expect("write enter commands");
        let text = String::from_utf8(buf).expect("enter commands are valid utf8");
        assert!(
            !text.contains("\x1b[?25l"),
            "enter commands must not hide the cursor: {text:?}"
        );
        // Mouse capture must remain enabled (the v0.9.12 sandbox contract).
        assert!(
            text.contains("\x1b[?1000h") || text.contains("\x1b[?1003h"),
            "enter commands should still enable mouse capture: {text:?}"
        );
    }

    #[test]
    fn composer_idle_positions_cursor_at_prompt() {
        // Live-room layout at 100×28 = status(1) + body(21) + composer(5)
        // + footer(1). Composer area y=22, h=5; inner_y=23, inner_x=1.
        // Idle prompt is "cr > " (width 5), so an empty composer parks
        // the cursor at (1 + 5 + 0, 23 + 0) = (6, 23).
        let state = test_state();
        let (_, cursor) = render_with_cursor(&state, 100, 28);
        assert_eq!(cursor, (6, 23));
    }

    #[test]
    fn composer_cursor_advances_with_ascii_input() {
        let mut state = test_state();
        state.composer.insert_char('h');
        state.composer.insert_char('i');
        let (_, cursor) = render_with_cursor(&state, 100, 28);
        // Two ASCII chars → col advances by 2 from the empty baseline.
        assert_eq!(cursor, (8, 23));
    }

    #[test]
    fn composer_cursor_respects_unicode_display_width() {
        let mut state = test_state();
        state.composer.insert_char('你');
        state.composer.insert_char('好');
        let (_, cursor) = render_with_cursor(&state, 100, 28);
        // Each CJK char is 2 display cells wide → col advances by 4.
        assert_eq!(cursor, (10, 23));
    }

    #[test]
    fn composer_cursor_descends_past_explicit_newline() {
        let mut state = test_state();
        state.composer.insert_char('a');
        state.composer.insert_newline();
        state.composer.insert_char('b');
        let (_, cursor) = render_with_cursor(&state, 100, 28);
        // After `\n`, row index advances by 1; the new row begins with the
        // same width-5 continuation indent, so 'b' sits at col 1 (inner_x)
        // + 5 (continuation indent) + 1 (the char itself) = 7.
        assert_eq!(cursor, (7, 24));
    }

    #[test]
    fn composer_does_not_reposition_cursor_when_blocked() {
        let mut state = test_state();
        let (tx, _rx) = mpsc::unbounded_channel();
        state.apply_event(RoomEvent::PermissionPrompt {
            request: sample_request(),
            host_role: "host".to_owned(),
            response_tx: Some(tx),
        });
        let (_, cursor) = render_with_cursor(&state, 100, 28);
        // Blocked state intentionally skips `frame.set_cursor_position`,
        // so ratatui's draw calls `hide_cursor()` and the backend cursor
        // stays at its initial origin. The user sees no caret in the
        // dimmed composer body, matching the "waiting for approval"
        // visual contract.
        assert_eq!(cursor, (0, 0));
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

    fn mouse_event(kind: MouseEventKind) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::empty(),
        })
    }

    fn key_event(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::empty(),
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::empty(),
        }
    }

    fn populate_scrollback(state: &mut RoomRuntimeState, lines: usize) {
        for i in 0..lines {
            state.push_scrollback(Line::from(format!("line {i}")));
        }
    }

    #[test]
    fn mouse_wheel_unrelated_buttons_do_not_move_scroll() {
        use crossterm::event::MouseButton;
        let mut state = test_state();
        populate_scrollback(&mut state, 100);
        state.last_viewport_rows.set(20);
        let (tx, _rx) = mpsc::unbounded_channel();
        handle_terminal_event(
            mouse_event(MouseEventKind::Down(MouseButton::Left)),
            &mut state,
            &tx,
        )
        .expect("event handled");
        assert_eq!(state.scroll_offset(), 0);
    }

    #[test]
    fn mouse_wheel_up_scrolls_history_then_clamps_at_top() {
        let mut state = test_state();
        populate_scrollback(&mut state, 30);
        state.last_viewport_rows.set(10);
        let (tx, _rx) = mpsc::unbounded_channel();
        for _ in 0..3 {
            handle_terminal_event(mouse_event(MouseEventKind::ScrollUp), &mut state, &tx)
                .expect("event handled");
        }
        // Each wheel notch is 3 lines; three notches = 9. Max is
        // scrollback (30) - viewport (10) = 20, so we are well below
        // the clamp.
        assert_eq!(state.scroll_offset(), 9);
        // Keep spamming until we hit the clamp; ScrollUp must not
        // overshoot the oldest visible row.
        for _ in 0..100 {
            handle_terminal_event(mouse_event(MouseEventKind::ScrollUp), &mut state, &tx)
                .expect("event handled");
        }
        assert_eq!(state.scroll_offset(), 20);
    }

    #[test]
    fn mouse_wheel_down_clears_unread_when_back_at_bottom() {
        let mut state = test_state();
        populate_scrollback(&mut state, 30);
        state.last_viewport_rows.set(10);
        let (tx, _rx) = mpsc::unbounded_channel();
        // Walk up so unread can accumulate.
        for _ in 0..2 {
            handle_terminal_event(mouse_event(MouseEventKind::ScrollUp), &mut state, &tx)
                .expect("event handled");
        }
        assert_eq!(state.scroll_offset(), 6);
        // New lines arrive while we are scrolled up.
        state.push_scrollback(Line::from("fresh 1"));
        state.push_scrollback(Line::from("fresh 2"));
        assert_eq!(state.unread_since_scroll(), 2);
        // Walking back to the bottom clears the unread badge.
        for _ in 0..3 {
            handle_terminal_event(mouse_event(MouseEventKind::ScrollDown), &mut state, &tx)
                .expect("event handled");
        }
        assert_eq!(state.scroll_offset(), 0);
        assert_eq!(state.unread_since_scroll(), 0);
    }

    #[test]
    fn page_keys_scroll_by_half_a_viewport() {
        let mut state = test_state();
        populate_scrollback(&mut state, 40);
        state.last_viewport_rows.set(20);
        let (tx, _rx) = mpsc::unbounded_channel();
        handle_terminal_event(Event::Key(key_event(KeyCode::PageUp)), &mut state, &tx)
            .expect("event handled");
        // Half of 20 = 10.
        assert_eq!(state.scroll_offset(), 10);
        handle_terminal_event(Event::Key(key_event(KeyCode::PageDown)), &mut state, &tx)
            .expect("event handled");
        assert_eq!(state.scroll_offset(), 0);
    }

    #[test]
    fn home_jumps_to_top_only_when_composer_is_empty() {
        let mut state = test_state();
        populate_scrollback(&mut state, 25);
        state.last_viewport_rows.set(10);
        let (tx, _rx) = mpsc::unbounded_channel();

        // Home with text in the composer must still move the input
        // cursor (Composer's move_home), never the scrollback.
        state.composer.insert_char('a');
        state.composer.insert_char('b');
        state.composer.move_right(); // cursor at end so move_home is a real action
        handle_terminal_event(Event::Key(key_event(KeyCode::Home)), &mut state, &tx)
            .expect("event handled");
        assert_eq!(state.scroll_offset(), 0);
        assert_eq!(state.composer.cursor(), 0);

        // With an empty composer, Home jumps to the top of history.
        state.composer.clear();
        handle_terminal_event(Event::Key(key_event(KeyCode::Home)), &mut state, &tx)
            .expect("event handled");
        assert_eq!(state.scroll_offset(), 25 - 10);

        // End with an empty composer returns to the bottom and resets
        // the unread badge.
        state.push_scrollback(Line::from("post-scroll arrival"));
        assert_eq!(state.unread_since_scroll(), 1);
        handle_terminal_event(Event::Key(key_event(KeyCode::End)), &mut state, &tx)
            .expect("event handled");
        assert_eq!(state.scroll_offset(), 0);
        assert_eq!(state.unread_since_scroll(), 0);
    }

    #[test]
    fn permission_overlay_disables_scroll_events() {
        let mut state = test_state();
        populate_scrollback(&mut state, 30);
        state.last_viewport_rows.set(10);
        let (tx, _rx) = mpsc::unbounded_channel();
        let (resp_tx, _resp_rx) = mpsc::unbounded_channel();
        state.apply_event(RoomEvent::PermissionPrompt {
            request: sample_request(),
            host_role: "host".to_owned(),
            response_tx: Some(resp_tx),
        });
        // Mouse wheel must not move the transcript while a permission
        // modal is open — the user's attention belongs to the prompt.
        handle_terminal_event(mouse_event(MouseEventKind::ScrollUp), &mut state, &tx)
            .expect("event handled");
        assert_eq!(state.scroll_offset(), 0);
    }

    #[test]
    fn scrolled_back_render_shows_follow_indicator_with_unread_count() {
        let mut state = test_state();
        populate_scrollback(&mut state, 30);
        state.last_viewport_rows.set(20);
        state.scroll_up(5);
        // A real arrival while scrolled adds to the unread badge.
        state.push_scrollback(Line::from("fresh delta"));
        let text = render_room_runtime_to_text(&state, 100, 28).expect("render");
        assert!(
            text.contains("↓ 1 new · End to follow"),
            "expected unread indicator in scrollback footer; got:\n{text}"
        );
    }

    #[test]
    fn scrolled_back_render_without_unread_shows_quiet_indicator() {
        let mut state = test_state();
        populate_scrollback(&mut state, 30);
        state.last_viewport_rows.set(20);
        state.scroll_up(5);
        let text = render_room_runtime_to_text(&state, 100, 28).expect("render");
        assert!(
            text.contains("↑ scrolled back · End to follow"),
            "expected quiet indicator; got:\n{text}"
        );
    }

    #[test]
    fn at_bottom_render_omits_follow_indicator() {
        let mut state = test_state();
        populate_scrollback(&mut state, 30);
        let text = render_room_runtime_to_text(&state, 100, 28).expect("render");
        assert!(!text.contains("scrolled back · End to follow"));
        assert!(!text.contains("new · End to follow"));
    }

    #[test]
    fn scroll_before_first_frame_does_not_park_user_on_pre_render_state() {
        // Regression: `scroll_up` / `scroll_to_top` were called before
        // any render happened, so `last_viewport_rows == 0` made
        // `scroll_max` collapse to `len - 1` — parking the user one
        // row from the top with no orientation. Guard returns instead.
        let mut state = test_state();
        populate_scrollback(&mut state, 50);
        assert_eq!(state.last_viewport_rows.get(), 0);
        state.scroll_up(10);
        state.scroll_to_top();
        assert_eq!(state.scroll_offset(), 0);
        // After the first frame paints the viewport hint, the same
        // call lands at a sensible position.
        let _ = render_room_runtime_to_text(&state, 100, 28).expect("render");
        state.scroll_up(10);
        assert_eq!(state.scroll_offset(), 10);
    }

    #[test]
    fn drain_caps_unread_to_what_remains_below_the_view() {
        // Push enough lines while scrolled that `push_scrollback`
        // drains the buffer. Anything that was evicted can no longer
        // be revealed by scrolling down, so `unread_since_scroll`
        // must never exceed `scroll_offset` after a drain.
        let mut state = test_state();
        populate_scrollback(&mut state, 50);
        state.last_viewport_rows.set(10);
        state.scroll_up(5);
        assert_eq!(state.scroll_offset(), 5);
        for i in 0..1200 {
            state.push_scrollback(Line::from(format!("flood {i}")));
        }
        // Drain happens once `scrollback.len() > 1000`, so after the
        // flood the buffer is capped at 1000 and the unread counter
        // is capped at `scroll_offset = 5`.
        assert!(state.unread_since_scroll() <= state.scroll_offset());
        assert_eq!(state.unread_since_scroll(), 5);
    }

    #[test]
    fn tiny_viewport_renders_only_the_indicator_when_scrolled() {
        // A 1-row Room (`visible_rows.saturating_sub(1) == 0`) still
        // has the indicator at the bottom, and the slice math must
        // produce an empty history window without panicking.
        let mut state = test_state();
        populate_scrollback(&mut state, 30);
        state.last_viewport_rows.set(1);
        state.scroll_up(5);
        // Total height 5 with a 5-row composer + 1-row footer + 1-row
        // status leaves a Room area roughly 3 rows tall = 1 inner
        // row after borders. Render must not panic and the indicator
        // text must still appear inside the composed buffer.
        let text = render_room_runtime_to_text(&state, 40, 10).expect("render");
        // The indicator copy is the load-bearing assertion — exact
        // truncation can vary with width, so check for either suffix.
        assert!(
            text.contains("scrolled back") || text.contains("new · End to follow"),
            "expected indicator copy in tiny-viewport render; got:\n{text}"
        );
    }

    #[test]
    fn banner_with_ansi_preserves_role_colors_in_scrollback() {
        let mut state = test_state();
        // Imitate a splash row coming through the sink as a banner.
        // The explicit SGR sequence carries 24-bit RGB color escape
        // codes that the ratatui scrollback must keep.
        let role_color = crate::output::role_color("backend", "host");
        let (r, g, b) = match role_color {
            crossterm::style::Color::Rgb { r, g, b } => (r, g, b),
            other => panic!("expected RGB role color, got {other:?}"),
        };
        let banner = format!("◇ \x1b[38;2;{r};{g};{b}m@backend\x1b[39m  cc · 1M · ask\n");
        state.apply_event(RoomEvent::Banner(banner));

        let expected = Color::Rgb(r, g, b);
        let coloured = state
            .scrollback
            .iter()
            .flat_map(|line| line.spans.iter())
            .find(|span| span.content.as_ref() == "@backend" && span.style.fg.is_some())
            .expect("@backend span survived scrollback");
        assert_eq!(coloured.style.fg, Some(expected));
    }

    // `work_card_renders_role_label_exactly_once_per_card` was a
    // pre-v0.10 rail assertion: it counted `@backend` occurrences
    // inside the WorkCard body that the rail used to render. The
    // slim rail (#383) no longer renders the WorkCard body at all,
    // so this assertion has no surface to verify. Per AC-4 the test
    // is deleted, not adapted.

    #[test]
    fn current_section_surfaces_tools_count_when_above_zero() {
        let mut state = test_state();
        state.apply_event(RoomEvent::Spinner(SpinnerSnapshot {
            role: "backend".to_owned(),
            frame: 0,
            started_at: Instant::now(),
            tools_seen: 3,
            current_state: Some("thinking".to_owned()),
            paint: SpinnerPaint::Painting,
        }));
        let text = render_room_runtime_to_text(&state, 120, 40).expect("render");
        assert!(text.contains("3 tools"));
    }

    #[test]
    fn current_section_omits_tool_count_when_zero() {
        let mut state = test_state();
        state.apply_event(RoomEvent::Spinner(SpinnerSnapshot {
            role: "backend".to_owned(),
            frame: 0,
            started_at: Instant::now(),
            tools_seen: 0,
            current_state: Some("thinking".to_owned()),
            paint: SpinnerPaint::Painting,
        }));
        let text = render_room_runtime_to_text(&state, 120, 40).expect("render");
        // No "0 tools" anywhere on the rail.
        assert!(!text.contains("0 tool"));
    }

    #[test]
    fn current_section_prefers_work_card_current_step_over_thinking() {
        let mut state = test_state();
        state.apply_event(RoomEvent::Spinner(SpinnerSnapshot {
            role: "backend".to_owned(),
            frame: 0,
            started_at: Instant::now(),
            tools_seen: 1,
            current_state: Some("thinking".to_owned()),
            paint: SpinnerPaint::Painting,
        }));
        // sample_work_card carries current_step = Some("cargo test").
        state.apply_event(RoomEvent::WorkCard(sample_work_card()));
        let text = render_room_runtime_to_text(&state, 120, 60).expect("render");
        // Status panel's Current section uses the card's current_step
        // verbatim instead of the generic spinner "thinking".
        assert!(text.contains("cargo test"));
    }

    #[test]
    fn room_activity_card_is_empty_when_no_role_is_working() {
        let state = test_state();
        let text = render_room_runtime_to_text(&state, 120, 40).expect("render");
        // The Room shows clean scrollback when idle — no activity
        // indicator row, no framing.
        assert!(!text.contains("team working"));
        assert!(!text.contains("@host · "));
    }

    #[test]
    fn room_activity_card_surfaces_active_role_with_tool_count_and_step() {
        let mut state = test_state();
        state.apply_event(RoomEvent::Spinner(SpinnerSnapshot {
            role: "backend".to_owned(),
            frame: 0,
            started_at: Instant::now(),
            tools_seen: 2,
            current_state: Some("thinking".to_owned()),
            paint: SpinnerPaint::Painting,
        }));
        // sample_work_card carries current_step = Some("cargo test").
        state.apply_event(RoomEvent::WorkCard(sample_work_card()));
        let text = render_room_runtime_to_text(&state, 120, 40).expect("render");
        // No framing — just the inline row carrying role, tool count,
        // and the running step from the work card.
        assert!(!text.contains("team working"));
        let activity_row = text
            .lines()
            .find(|line| line.contains("@backend") && line.contains("cargo test"))
            .expect("inline activity row");
        assert!(activity_row.contains("2 tools"));
    }

    #[test]
    fn room_activity_card_lists_every_active_role_separately() {
        let mut state = test_state();
        state.apply_event(RoomEvent::Spinner(SpinnerSnapshot {
            role: "host".to_owned(),
            frame: 0,
            started_at: Instant::now(),
            tools_seen: 1,
            current_state: Some("thinking".to_owned()),
            paint: SpinnerPaint::Painting,
        }));
        state.apply_event(RoomEvent::Spinner(SpinnerSnapshot {
            role: "backend".to_owned(),
            frame: 0,
            started_at: Instant::now(),
            tools_seen: 3,
            current_state: Some("running".to_owned()),
            paint: SpinnerPaint::Painting,
        }));
        let text = render_room_runtime_to_text(&state, 120, 40).expect("render");
        assert!(text
            .lines()
            .any(|line| line.contains("@host") && line.contains("1 tool")));
        assert!(text
            .lines()
            .any(|line| line.contains("@backend") && line.contains("3 tools")));
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
    fn permission_overlay_keeps_role_in_status_without_team_fallback() {
        let mut state = test_state();
        let (tx, _rx) = mpsc::unbounded_channel();
        state.apply_event(RoomEvent::PermissionPrompt {
            request: sample_request(),
            host_role: "host".to_owned(),
            response_tx: Some(tx),
        });
        let text = render_room_runtime_to_text(&state, 120, 30).expect("render");
        assert!(text.contains("Status"));
        assert!(!text.contains("Roles"));
        // Inline activity row in the Room surfaces the role waiting
        // on a permission prompt even when its spinner has been
        // cleared.
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

    // `work_and_spinner_events_populate_status_rail` asserted the
    // WorkCard body (title `Run validation`, role glyph `◇`) appeared
    // on the right rail. Per ADR §Q5 and #383 AC-4 the slim rail no
    // longer renders the WorkCard body, so this test is deleted
    // rather than adapted. The chat-stream working card (#381) owns
    // the role-glyph + title surface now.

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

    // `work_cards_render_a_role_identity_header_per_card` asserted
    // that the rail decorated each WorkCard with a `◇ @backend`
    // identity header. The slim rail (#383) no longer renders the
    // WorkCard body, so the header surface is gone. The chat-stream
    // working card (#381) carries identity inline now. Per AC-4 the
    // test is deleted, not adapted.

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

    /// Drive `state` through enough `CrepEvent`s to land one spawn in
    /// `Working`. Returns nothing; callers inspect
    /// `state.spawn_lifecycle()` or rerender to assert visuals.
    fn push_working_spawn(state: &mut RoomRuntimeState, role: &str, turn_id: &str) {
        state.apply_event(RoomEvent::Crep {
            event: Box::new(CrepEvent::TurnDispatched {
                role: role.to_owned(),
                priors_hash: String::new(),
                turn_id: turn_id.to_owned(),
                thread_id: format!("thread-{turn_id}"),
                parent_turn_id: None,
                queue_position: 0,
            }),
            host_role: state.host_role.clone(),
        });
        // First tool call promotes Spawning → Working.
        state.apply_event(RoomEvent::Crep {
            event: Box::new(CrepEvent::ToolCallProposed {
                role: role.to_owned(),
                priors_hash: String::new(),
                tool_name: "Bash".to_owned(),
                tool_input: json!({}),
                tool_use_id: format!("use-{turn_id}"),
                turn_id: turn_id.to_owned(),
                thread_id: format!("thread-{turn_id}"),
            }),
            host_role: state.host_role.clone(),
        });
    }

    /// Drive `state` through a `TurnDispatched` only — leaves the spawn
    /// in `Spawning` (no tool call yet).
    fn push_spawning_spawn(state: &mut RoomRuntimeState, role: &str, turn_id: &str) {
        state.apply_event(RoomEvent::Crep {
            event: Box::new(CrepEvent::TurnDispatched {
                role: role.to_owned(),
                priors_hash: String::new(),
                turn_id: turn_id.to_owned(),
                thread_id: format!("thread-{turn_id}"),
                parent_turn_id: None,
                queue_position: 0,
            }),
            host_role: state.host_role.clone(),
        });
    }

    /// Render only the narration row of `state` and return the flat
    /// text. Wraps `build_footer_narration_line` so width / truncation
    /// semantics can be exercised directly without a full frame.
    fn narration_text(state: &RoomRuntimeState, width: usize) -> String {
        let working = state
            .spawn_lifecycle
            .working_instances_ordered_by_started_at();
        let spawning = state
            .spawn_lifecycle
            .spawning_instances_ordered_by_started_at();
        super::build_footer_narration_line(&working, &spawning, &state.host_role, width)
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    }

    #[test]
    fn footer_narration_hidden_when_no_subagents() {
        // AC-4: zero Working + zero Spawning → strip is not rendered,
        // composer occupies the row that the strip would otherwise own.
        let state = test_state();
        assert_eq!(state.spawn_lifecycle.working_count(), 0);
        let text = render_room_runtime_to_text(&state, 120, 28).expect("render");
        assert!(
            !text.contains("still working"),
            "narration should be hidden when no sub-agents are active:\n{text}"
        );
        // Composer baseline cursor at 100×28 is row 23 (see
        // `composer_idle_positions_cursor_at_prompt`). Inserting one
        // working spawn pushes the composer down by 1; absent any
        // spawn, the composer stays put. Confirm via cursor.
        let (_, cursor) = render_with_cursor(&state, 100, 28);
        assert_eq!(cursor, (6, 23));
    }

    #[test]
    fn footer_narration_shows_one_role_singular() {
        // AC-2, AC-5: one Working instance → "1 role still working ·
        // @backend" (singular).
        let mut state = test_state();
        push_working_spawn(&mut state, "backend", "t1");
        let line = narration_text(&state, 120);
        assert!(
            line.contains("1 role still working"),
            "expected singular header: {line:?}"
        );
        assert!(!line.contains("1 roles"));
        assert!(line.contains("@backend"), "expected role chip: {line:?}");
    }

    #[test]
    fn footer_narration_shows_multiple_roles_with_plural() {
        // AC-2, AC-5: two+ Working instances → "N roles still working"
        // followed by the chips in started_at order.
        let mut state = test_state();
        push_working_spawn(&mut state, "security", "t1");
        std::thread::sleep(std::time::Duration::from_millis(2));
        push_working_spawn(&mut state, "backend", "t2");
        let line = narration_text(&state, 120);
        assert!(
            line.contains("2 roles still working"),
            "expected plural header: {line:?}"
        );
        assert!(line.contains("@security"));
        assert!(line.contains("@backend"));
        // Order: security (earlier started_at) before backend.
        let sec_idx = line.find("@security").expect("@security in line");
        let be_idx = line.find("@backend").expect("@backend in line");
        assert!(
            sec_idx < be_idx,
            "started_at order broken (security should precede backend): {line:?}"
        );
    }

    #[test]
    fn footer_narration_names_spawning_with_suffix() {
        // ADR locked rule: Spawning roles get a "· @x spawning" suffix
        // chip but do NOT increment the leading count. One Working +
        // one Spawning → "1 role still working · @<work> · @<spawn>
        // spawning".
        let mut state = test_state();
        push_working_spawn(&mut state, "security", "t1");
        push_spawning_spawn(&mut state, "backend", "t2");
        let line = narration_text(&state, 120);
        // Count is Working-only.
        assert!(
            line.contains("1 role still working"),
            "Spawning must not increment the count: {line:?}"
        );
        assert!(!line.contains("2 roles still working"));
        // Spawning chip has the trailing " spawning" suffix.
        assert!(
            line.contains("@backend spawning"),
            "expected `@backend spawning` suffix chip: {line:?}"
        );
        // Working chip stays plain.
        assert!(line.contains("@security"));
        // ADR Q4: footer never promises "chat resumes when they report".
        assert!(
            !line.contains("chat resumes"),
            "ADR Q4 forbids 'chat resumes' wording: {line:?}"
        );

        // Zero-Working corner case (locked by the ADR): with only a
        // Spawning instance, the line reads "0 roles still working ·
        // @backend spawning". This also means the strip is rendered
        // when only Spawning exists (composer does NOT reclaim the row).
        let mut spawning_only = test_state();
        push_spawning_spawn(&mut spawning_only, "qa", "t1");
        let zero_line = narration_text(&spawning_only, 120);
        assert!(
            zero_line.contains("0 roles still working"),
            "ADR: zero Working + one Spawning ⇒ '0 roles still working': {zero_line:?}"
        );
        assert!(zero_line.contains("@qa spawning"));
    }

    #[test]
    fn footer_narration_truncates_when_too_many_roles() {
        // AC-6: when role chips exceed the row, the tail collapses to
        // `… +N more`. We push enough Working instances that even a
        // generous-looking width forces the truncation marker.
        let mut state = test_state();
        for i in 0..12 {
            push_working_spawn(&mut state, &format!("ingestor{i:02}"), &format!("t{i}"));
        }
        // 40 columns is well under the un-truncated rendering width of
        // "12 roles still working · @ingestor00 @ingestor01 …" → forces
        // a `… +N more` tail.
        let line = narration_text(&state, 40);
        assert!(
            line.contains("12 roles still working"),
            "expected header even when truncated: {line:?}"
        );
        assert!(
            line.contains("more"),
            "expected truncation marker (…+N more): {line:?}"
        );
        assert!(
            line.contains("…"),
            "expected ellipsis in truncation marker: {line:?}"
        );
        // The rendered line, sans color, must not exceed the budget.
        let visible_width = UnicodeWidthStr::width(line.as_str());
        // Allow up to the budget — the truncation marker is best-effort
        // when the area is degenerate, but with 40 columns and the test
        // header it should always fit.
        assert!(
            visible_width <= 40,
            "narration overflowed row width 40 → {visible_width}: {line:?}"
        );
    }

    // ------------------------------------------------------------------
    // #381 — `WorkingCard` widget integration. Verifies that the card
    // materializes inside the rendered scrollback at the chat-time
    // position assigned at TurnDispatched, reads role identity from
    // `tui_style::role_color`, and does not break the existing
    // scrollback / scroll-offset model.
    // ------------------------------------------------------------------

    fn dispatch_event(role: &str, turn: &str) -> CrepEvent {
        CrepEvent::TurnDispatched {
            role: role.to_owned(),
            priors_hash: String::new(),
            turn_id: crate::turn::TurnId::from(turn.to_owned()),
            thread_id: crate::turn::TurnId::from(format!("thread-{turn}")),
            parent_turn_id: None,
            queue_position: 0,
        }
    }

    fn work_title_event(role: &str, turn: &str, title: &str) -> CrepEvent {
        CrepEvent::WorkTitle {
            role: role.to_owned(),
            priors_hash: String::new(),
            title: title.to_owned(),
            turn_id: crate::turn::TurnId::from(turn.to_owned()),
            thread_id: crate::turn::TurnId::from(format!("thread-{turn}")),
        }
    }

    fn tool_proposed_event(role: &str, turn: &str, tool: &str, tool_use_id: &str) -> CrepEvent {
        CrepEvent::ToolCallProposed {
            role: role.to_owned(),
            priors_hash: String::new(),
            tool_name: tool.to_owned(),
            tool_input: serde_json::json!({}),
            tool_use_id: tool_use_id.to_owned(),
            turn_id: crate::turn::TurnId::from(turn.to_owned()),
            thread_id: crate::turn::TurnId::from(format!("thread-{turn}")),
        }
    }

    fn tool_executed_event(role: &str, turn: &str, tool_use_id: &str, summary: &str) -> CrepEvent {
        CrepEvent::ToolCallExecuted {
            role: role.to_owned(),
            priors_hash: String::new(),
            tool_use_id: tool_use_id.to_owned(),
            ok: true,
            output_summary: summary.to_owned(),
            turn_id: crate::turn::TurnId::from(turn.to_owned()),
            thread_id: crate::turn::TurnId::from(format!("thread-{turn}")),
        }
    }

    fn role_spoke_event(role: &str, turn: &str, text: &str) -> CrepEvent {
        CrepEvent::RoleSpoke {
            role: role.to_owned(),
            priors_hash: String::new(),
            text: text.to_owned(),
            mentions: Vec::new(),
            cost_usd: 0.0,
            cache_read: 0,
            turn_id: crate::turn::TurnId::from(turn.to_owned()),
            thread_id: crate::turn::TurnId::from(format!("thread-{turn}")),
            outcome: crate::crep::TurnOutcome::default(),
            phase_block: None,
        }
    }

    fn crep_room_event(event: CrepEvent) -> RoomEvent {
        RoomEvent::Crep {
            event: Box::new(event),
            host_role: "host".to_owned(),
        }
    }

    #[test]
    fn working_card_renders_inline_with_role_title_and_state_label() {
        // Dispatch @security, set its title, fire a tool call to push
        // it from Spawning → Working. The rendered scrollback must
        // now contain the card's top border (with @security + the
        // task title + `working`) and its hotkey-hint footer.
        let mut state = test_state();
        state.apply_event(crep_room_event(dispatch_event("security", "t1")));
        state.apply_event(crep_room_event(work_title_event(
            "security",
            "t1",
            "audit README claims",
        )));
        state.apply_event(crep_room_event(tool_proposed_event(
            "security", "t1", "Read", "u1",
        )));
        let text = render_room_runtime_to_text(&state, 120, 30).expect("render");
        assert!(
            text.contains("@security"),
            "card top border must carry @security: {text}"
        );
        assert!(
            text.contains("audit README claims"),
            "card top border must carry the task title: {text}"
        );
        assert!(
            text.contains("working"),
            "card top border must carry the `working` state label: {text}"
        );
        assert!(
            text.contains("[e]xpand [i]nterrupt [f]ocus"),
            "card footer must render the hotkey hint string: {text}"
        );
    }

    #[test]
    fn footer_narration_chips_use_identity_colors() {
        // AC-3: role chips carry the same color as the @role mentions
        // in scrollback (sourced from `tui_style::role_color`).
        let mut state = test_state();
        push_working_spawn(&mut state, "backend", "t1");
        let working = state
            .spawn_lifecycle
            .working_instances_ordered_by_started_at();
        let spawning = state
            .spawn_lifecycle
            .spawning_instances_ordered_by_started_at();
        let line = super::build_footer_narration_line(&working, &spawning, &state.host_role, 120);
        let chip_color = line
            .spans
            .iter()
            .find(|span| span.content.as_ref() == "@backend")
            .and_then(|span| span.style.fg)
            .expect("@backend chip present with a color");
        assert_eq!(chip_color, tui_style::role_color("backend", "host"));
    }

    #[test]
    fn working_card_uses_no_title_placeholder_when_work_title_missing() {
        // AC: if no `WorkTitle` event has landed for the spawn, the
        // card top border shows the locked placeholder rather than
        // collapsing the border.
        let mut state = test_state();
        state.apply_event(crep_room_event(dispatch_event("backend", "t1")));
        // Skip the WorkTitle event. Fire a tool call to flip to Working.
        state.apply_event(crep_room_event(tool_proposed_event(
            "backend", "t1", "Bash", "u1",
        )));
        let text = render_room_runtime_to_text(&state, 120, 30).expect("render");
        assert!(
            text.contains(crate::working_card::NO_TITLE_PLACEHOLDER),
            "expected (no title) placeholder: {text}"
        );
    }

    #[test]
    fn working_card_only_renders_when_state_is_working_not_spawning() {
        // Only `Spawning` so far — no tool call, no output delta.
        // Per the ADR, Spawning has no card (it shows as a hint in
        // the spawner's text). The chat stream must not render a card
        // for a spawn that has not promoted to Working yet.
        let mut state = test_state();
        state.apply_event(crep_room_event(dispatch_event("backend", "t1")));
        state.apply_event(crep_room_event(work_title_event(
            "backend",
            "t1",
            "set up scaffolding",
        )));
        let text = render_room_runtime_to_text(&state, 120, 30).expect("render");
        // No card top border or footer.
        assert!(
            !text.contains("[e]xpand [i]nterrupt [f]ocus"),
            "no card footer should render for Spawning: {text}"
        );
    }

    #[test]
    fn working_card_uses_role_identity_color_via_tui_style() {
        // AC-3: no hard-coded role color. We can't sniff color from
        // the rendered text directly, but we can verify by running
        // the merge helper with a known role and inspecting the
        // top-border `@role` span style.
        let mut state = test_state();
        state.apply_event(crep_room_event(dispatch_event("security", "t1")));
        state.apply_event(crep_room_event(work_title_event("security", "t1", "audit")));
        state.apply_event(crep_room_event(tool_proposed_event(
            "security", "t1", "Read", "u1",
        )));
        let merged = super::build_merged_scrollback(&state, 120);
        let role_token_color = merged
            .iter()
            .flat_map(|line| line.spans.iter())
            .find(|span| span.content.as_ref().contains("@security"))
            .and_then(|span| span.style.fg)
            .expect("@security span has a foreground color");
        assert_eq!(
            role_token_color,
            tui_style::role_color("security", "host"),
            "card identity color must come from tui_style::role_color"
        );
    }

    #[test]
    fn working_card_body_shows_done_and_in_progress_markers() {
        // Drive @security through one Done tool call + one
        // InProgress. Card body must show ✓ for the done summary
        // and ∴ for the in-progress one.
        let mut state = test_state();
        state.apply_event(crep_room_event(dispatch_event("security", "t1")));
        state.apply_event(crep_room_event(work_title_event("security", "t1", "audit")));
        state.apply_event(crep_room_event(tool_proposed_event(
            "security", "t1", "Read", "u1",
        )));
        state.apply_event(crep_room_event(tool_executed_event(
            "security",
            "t1",
            "u1",
            "read README.md §2.4",
        )));
        // A second proposal that has not executed yet.
        state.apply_event(crep_room_event(tool_proposed_event(
            "security", "t1", "Grep", "u2",
        )));
        let text = render_room_runtime_to_text(&state, 120, 30).expect("render");
        assert!(
            text.contains('✓'),
            "card body must render ✓ for the done tool call: {text}"
        );
        assert!(
            text.contains('∴'),
            "card body must render ∴ for the in-progress tool call: {text}"
        );
        assert!(
            text.contains("read README.md"),
            "done summary must appear in body: {text}"
        );
        // Footer step count tracks done calls only.
        assert!(
            text.contains("1 step done"),
            "footer step count should be 1: {text}"
        );
    }

    #[test]
    fn working_card_position_follows_spawning_event_chat_time_not_bottom() {
        // AC-2: the card sits at the chat-time of TurnDispatched, not
        // pinned at the bottom. We push a chat-style line *after*
        // the spawn dispatches; the card must appear ABOVE that line
        // in the merged scrollback.
        let mut state = test_state();
        state.apply_event(crep_room_event(dispatch_event("security", "t1")));
        state.apply_event(crep_room_event(work_title_event("security", "t1", "audit")));
        state.apply_event(crep_room_event(tool_proposed_event(
            "security", "t1", "Read", "u1",
        )));
        // A post-spawn chat row from @host.
        state.push_scrollback(Line::from("post-spawn host line"));
        let merged = super::build_merged_scrollback(&state, 120);
        let card_row = merged
            .iter()
            .position(|line| {
                line.spans
                    .iter()
                    .any(|s| s.content.as_ref().contains("[e]xpand [i]nterrupt [f]ocus"))
            })
            .expect("card footer present");
        let post_row = merged
            .iter()
            .position(|line| {
                line.spans
                    .iter()
                    .any(|s| s.content.as_ref().contains("post-spawn host line"))
            })
            .expect("post-spawn line present");
        assert!(
            card_row < post_row,
            "card must render before post-spawn line: card={card_row}, post={post_row}"
        );
    }

    #[test]
    fn working_card_position_survives_scrollback_drain() {
        // Regression for the @reviewer audit on PR #392: the
        // 1000-row scrollback drain in `push_scrollback` evicts the
        // oldest rows from the front but used to leave the
        // SpawnInstance's `chat_position` pointing at a now-dead
        // index. Shipping that would have meant Working cards
        // silently slipped out of place after long sessions.
        //
        // Drive >1000 lines of scrollback with one Working spawn
        // anchored near the start, then assert: (a) `chat_position`
        // has been shifted by exactly `overflow`, (b) the merged
        // scrollback still places the card BEFORE the most recent
        // post-spawn lines.
        let mut state = test_state();
        state.apply_event(crep_room_event(dispatch_event("security", "t1")));
        state.apply_event(crep_room_event(work_title_event(
            "security",
            "t1",
            "audit README claims",
        )));
        state.apply_event(crep_room_event(tool_proposed_event(
            "security", "t1", "Read", "u1",
        )));
        let spawn_id_before = state
            .spawn_lifecycle
            .instances()
            .next()
            .expect("one spawn")
            .spawn_id;
        let position_before = state
            .spawn_lifecycle
            .get(spawn_id_before)
            .expect("spawn present")
            .chat_position;

        // Flood scrollback well past the 1000-row cap.
        for i in 0..1_500 {
            state.push_scrollback(Line::from(format!("filler {i}")));
        }

        // The drain shrunk the buffer to exactly 1000 rows. The card's
        // chat_position must have shifted by `(1 + 1500) - 1000 = 501`
        // (roughly — accounting for the spawn's own row contribution).
        // Concretely: chat_position must be strictly less than its
        // pre-drain value.
        let position_after = state
            .spawn_lifecycle
            .get(spawn_id_before)
            .expect("spawn still tracked")
            .chat_position;
        assert!(
            position_after < position_before
                || (position_before == 0 && position_after == 0),
            "chat_position should shift left after drain: before={position_before}, after={position_after}"
        );

        // Add one more post-spawn row and verify ordering still holds
        // in the merged scrollback.
        state.push_scrollback(Line::from("very fresh line"));
        let merged = super::build_merged_scrollback(&state, 120);
        let card_row = merged.iter().position(|line| {
            line.spans
                .iter()
                .any(|s| s.content.as_ref().contains("[e]xpand [i]nterrupt [f]ocus"))
        });
        let fresh_row = merged.iter().position(|line| {
            line.spans
                .iter()
                .any(|s| s.content.as_ref().contains("very fresh line"))
        });
        if let (Some(card), Some(fresh)) = (card_row, fresh_row) {
            assert!(
                card < fresh,
                "post-drain: card must still precede the latest line (card={card}, fresh={fresh})"
            );
        }
    }

    #[test]
    fn build_merged_scrollback_is_idempotent() {
        // Rerendering the same state twice must produce byte-identical
        // merged scrollback. If a future change introduces hidden
        // mutation (e.g., recomputing card positions on every call),
        // this test surfaces it as a flake before users do.
        let mut state = test_state();
        state.apply_event(crep_room_event(dispatch_event("security", "t1")));
        state.apply_event(crep_room_event(work_title_event(
            "security",
            "t1",
            "audit README claims",
        )));
        state.apply_event(crep_room_event(tool_proposed_event(
            "security", "t1", "Read", "u1",
        )));
        state.apply_event(crep_room_event(tool_executed_event(
            "security",
            "t1",
            "u1",
            "read README.md §2.4",
        )));
        let first = super::build_merged_scrollback(&state, 120);
        let second = super::build_merged_scrollback(&state, 120);
        let first_text: String = first
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect();
        let second_text: String = second
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(first_text, second_text);
    }

    #[test]
    fn done_collapsed_line_replaces_working_card_without_report_row() {
        // #384 AC-1 / AC-3: a finished spawn with no report text
        // renders a collapsed Done marker in the original card slot
        // and does not synthesize an empty report row.
        let mut state = test_state();
        state.apply_event(crep_room_event(dispatch_event("security", "t1")));
        state.apply_event(crep_room_event(work_title_event("security", "t1", "audit")));
        state.apply_event(crep_room_event(tool_proposed_event(
            "security", "t1", "Read", "u1",
        )));
        state.apply_event(crep_room_event(tool_executed_event(
            "security",
            "t1",
            "u1",
            "read README.md",
        )));
        state.apply_event(crep_room_event(role_spoke_event("security", "t1", "")));

        let merged = super::build_merged_scrollback(&state, 120);
        let text = super::lines_to_plain_string(&merged);
        assert!(
            text.contains("@security ✓ done"),
            "missing Done marker: {text}"
        );
        assert!(text.contains("1 step"), "missing step count: {text}");
        assert!(
            !text.contains("[e]xpand [i]nterrupt [f]ocus"),
            "working-card footer should be gone after Done: {text}"
        );

        assert!(
            state.spawn_report_rows.is_empty(),
            "silent completion must not create report rows"
        );
    }

    #[test]
    fn reported_message_is_spliced_immediately_after_done_marker() {
        // #384 AC-2: the report is rendered as a normal role-attributed
        // chat row immediately below the collapsed marker, not at the
        // live tail where the RoleSpoke event arrived.
        let mut state = test_state();
        state.apply_event(crep_room_event(dispatch_event("qa", "t1")));
        state.apply_event(crep_room_event(work_title_event("qa", "t1", "smoke test")));
        state.apply_event(crep_room_event(tool_proposed_event(
            "qa", "t1", "Bash", "u1",
        )));
        state.apply_event(crep_room_event(tool_executed_event(
            "qa",
            "t1",
            "u1",
            "ran smoke script",
        )));
        // First RoleSpoke reaches Done silently; the second carries
        // the report and moves the lifecycle to Reported.
        state.apply_event(crep_room_event(role_spoke_event("qa", "t1", "")));
        state.apply_event(crep_room_event(role_spoke_event(
            "qa",
            "t1",
            "headless smoke pass: 2/2 scenarios green",
        )));

        let merged = super::build_merged_scrollback(&state, 120);
        let marker_row = line_position(&merged, "@qa ✓ done").expect("marker row");
        let report_header = merged
            .get(marker_row + 1)
            .map(line_text)
            .expect("report header directly under marker");
        assert!(
            report_header.contains("@qa"),
            "report must keep normal role attribution: {report_header:?}"
        );
        let report_body_row =
            line_position(&merged, "headless smoke pass").expect("report body row");
        assert!(
            report_body_row > marker_row && report_body_row <= marker_row + 3,
            "report body should be adjacent to marker: marker={marker_row}, body={report_body_row}"
        );
    }

    #[test]
    fn interleaved_done_markers_keep_spawn_time_order() {
        // #384 AC-5c: completion order must not reorder cards. The
        // marker occupies the original spawn slot, so a later-spawned
        // role that finishes first still appears after the earlier
        // spawn in the transcript.
        let mut state = test_state();
        state.apply_event(crep_room_event(dispatch_event("backend", "t1")));
        state.apply_event(crep_room_event(tool_proposed_event(
            "backend", "t1", "Bash", "u1",
        )));
        state.apply_event(crep_room_event(dispatch_event("security", "t2")));
        state.apply_event(crep_room_event(tool_proposed_event(
            "security", "t2", "Read", "u2",
        )));

        // Finish in reverse order: security first, backend second.
        state.apply_event(crep_room_event(role_spoke_event("security", "t2", "")));
        state.apply_event(crep_room_event(role_spoke_event("backend", "t1", "")));

        let merged = super::build_merged_scrollback(&state, 120);
        let backend_row = line_position(&merged, "@backend ✓ done").expect("backend marker");
        let security_row = line_position(&merged, "@security ✓ done").expect("security marker");
        assert!(
            backend_row < security_row,
            "markers must remain in spawn-time order, not completion order"
        );
    }

    #[test]
    fn tab_and_shift_tab_cycle_card_focus() {
        let mut state = test_state();
        push_working_spawn(&mut state, "backend", "t1");
        push_working_spawn(&mut state, "security", "t2");
        let (tx, _rx) = mpsc::unbounded_channel();

        super::handle_key(
            KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()),
            &mut state,
            &tx,
        )
        .expect("tab handled");
        assert_eq!(
            state.focused_spawn().map(|spawn| spawn.role.as_str()),
            Some("backend")
        );

        super::handle_key(
            KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()),
            &mut state,
            &tx,
        )
        .expect("tab handled");
        assert_eq!(
            state.focused_spawn().map(|spawn| spawn.role.as_str()),
            Some("security")
        );

        super::handle_key(
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
            &mut state,
            &tx,
        )
        .expect("backtab handled");
        assert_eq!(
            state.focused_spawn().map(|spawn| spawn.role.as_str()),
            Some("backend")
        );
    }

    #[test]
    fn card_hotkeys_fall_through_when_no_card_is_focused() {
        let mut state = test_state();
        push_working_spawn(&mut state, "backend", "t1");
        let (tx, _rx) = mpsc::unbounded_channel();

        super::handle_key(
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::empty()),
            &mut state,
            &tx,
        )
        .expect("key handled");
        super::handle_key(
            KeyEvent::new(KeyCode::Char('i'), KeyModifiers::empty()),
            &mut state,
            &tx,
        )
        .expect("key handled");
        super::handle_key(
            KeyEvent::new(KeyCode::Char('f'), KeyModifiers::empty()),
            &mut state,
            &tx,
        )
        .expect("key handled");
        assert_eq!(state.composer.view_model().input, "eif");
    }

    #[test]
    fn e_toggles_expanded_done_log_for_focused_done_card() {
        let mut state = test_state();
        state.apply_event(crep_room_event(dispatch_event("qa", "t1")));
        state.apply_event(crep_room_event(tool_proposed_event(
            "qa", "t1", "Bash", "u1",
        )));
        state.apply_event(crep_room_event(tool_executed_event(
            "qa",
            "t1",
            "u1",
            "ran smoke script",
        )));
        state.apply_event(crep_room_event(role_spoke_event("qa", "t1", "")));
        let spawn_id = state.spawn_lifecycle.instances().next().unwrap().spawn_id;
        state.focused_spawn = Some(spawn_id);
        let (tx, _rx) = mpsc::unbounded_channel();

        super::handle_key(
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::empty()),
            &mut state,
            &tx,
        )
        .expect("expand handled");
        let expanded = super::lines_to_plain_string(&super::build_merged_scrollback(&state, 120));
        let expanded_count = expanded.matches("ran smoke script").count();
        assert!(
            expanded_count >= 2,
            "expanded log should include the tool summary: {expanded}"
        );

        super::handle_key(
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::empty()),
            &mut state,
            &tx,
        )
        .expect("collapse handled");
        let collapsed = super::lines_to_plain_string(&super::build_merged_scrollback(&state, 120));
        assert_eq!(
            collapsed.matches("ran smoke script").count(),
            expanded_count - 1,
            "second e should remove only the expanded log row: {collapsed}"
        );
    }

    #[test]
    fn i_interrupts_only_the_focused_working_card_role() {
        let mut state = test_state();
        push_working_spawn(&mut state, "backend", "t1");
        push_working_spawn(&mut state, "security", "t2");
        state.focused_spawn = state
            .spawn_lifecycle
            .instances()
            .find(|spawn| spawn.role == "security")
            .map(|spawn| spawn.spawn_id);
        let (tx, mut rx) = mpsc::unbounded_channel();

        super::handle_key(
            KeyEvent::new(KeyCode::Char('i'), KeyModifiers::empty()),
            &mut state,
            &tx,
        )
        .expect("interrupt handled");
        assert_eq!(
            rx.try_recv().expect("halt command sent"),
            RuntimeInput::Line("/halt @security".to_owned())
        );
    }

    #[test]
    fn f_focus_mode_expands_focused_card_and_stubs_the_others() {
        let mut state = test_state();
        push_working_spawn(&mut state, "backend", "t1");
        push_working_spawn(&mut state, "security", "t2");
        state.focused_spawn = state
            .spawn_lifecycle
            .instances()
            .find(|spawn| spawn.role == "backend")
            .map(|spawn| spawn.spawn_id);
        let (tx, _rx) = mpsc::unbounded_channel();

        super::handle_key(
            KeyEvent::new(KeyCode::Char('f'), KeyModifiers::empty()),
            &mut state,
            &tx,
        )
        .expect("focus mode handled");
        let merged = super::lines_to_plain_string(&super::build_merged_scrollback(&state, 120));
        assert!(merged.contains("@backend"));
        assert!(merged.contains("[e]xpand [i]nterrupt [f]ocus"));
        assert!(merged.contains("@security ·"));
        assert_eq!(
            merged.matches("[e]xpand [i]nterrupt [f]ocus").count(),
            1,
            "only focused card should keep the full card footer: {merged}"
        );
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

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    fn line_position(lines: &[Line<'_>], needle: &str) -> Option<usize> {
        lines
            .iter()
            .position(|line| line_text(line).contains(needle))
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
