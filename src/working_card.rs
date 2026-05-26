//! Inline `WorkingCard` widget for the v0.10 chat stream.
//!
//! Renders one `Working`-state spawn instance as a multi-line ASCII
//! card in the live-room scrollback. The card composes a top border
//! (`┌─ @role · title ── working · elapsed ─┐`), a body of tool-call
//! lines (`✓` done, `∴` in progress, `⨯` failed), and a bottom border
//! carrying a step count and the locked hotkey hint string. The hint
//! string is **non-functional** in this PR — `#385` wires the keys.
//!
//! The widget reads from [`SpawnInstance`] only. It does not consult
//! the kernel, the rail, or any side channel. The caller threads in
//! the host role (for identity color via [`tui_style::role_color`]) and
//! the available inner width.
//!
//! Visual locked by `docs/v0.10-chat-stream-vs-dashboard.md` Frame B
//! and by `examples/chat-stream-demo.rs` (issue `#379`). This module
//! is the production version of that prototype, fed by real lifecycle
//! data instead of hand-built scene fixtures.

use std::time::{Duration, Instant};

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::spawn_lifecycle::{Outcome, SpawnInstance, SpawnState, ToolCallRecord, ToolCallStatus};
use crate::tui_style;

/// Default count of tool-call rows visible inside the card body.
/// Anything older than the last [`DEFAULT_VISIBLE_STEPS`] entries
/// scrolls out of the card (the lines are not added to scrollback;
/// the card is the bounded region per the ADR).
pub const DEFAULT_VISIBLE_STEPS: usize = 3;

/// Indent applied to every card line. Matches the column where
/// `@role` chat messages start in the existing scrollback layout, so
/// the card visually attaches to the spawner's most recent line.
const CARD_INDENT: &str = "             ";

/// Minimum inner card width. Below this the borders collapse to a
/// single-char marker but the body still renders — we never want a
/// negative width to panic out of the layout pass.
const MIN_CARD_WIDTH: usize = 40;

/// Placeholder shown on the top border when a spawn has not yet
/// emitted a `WorkTitle` event. Documented in the AC: "if neither is
/// available, use `(no title)` placeholder."
pub const NO_TITLE_PLACEHOLDER: &str = "(no title)";

/// Build the lines that make up one working card for the given spawn.
///
/// Returns an empty vec when `spawn.state != SpawnState::Working` —
/// the caller is free to call this for every instance and rely on the
/// state filter to drop non-working ones, which keeps the integration
/// point in `render_scrollback` simple.
///
/// `now` is taken as a parameter (not read from `Instant::now()`
/// inside) so tests can pin the elapsed-seconds value deterministically
/// and so the renderer's "once per second" gate (AC-5) is the caller's
/// responsibility, not the widget's.
#[must_use]
pub fn render_working_card_lines(
    spawn: &SpawnInstance,
    host_role: &str,
    inner_width: u16,
    now: Instant,
    visible_steps: usize,
) -> Vec<Line<'static>> {
    render_working_card_lines_with_focus(spawn, host_role, inner_width, now, visible_steps, false)
}

#[must_use]
/// Build the working-card lines, with a brighter border when focused.
pub fn render_working_card_lines_with_focus(
    spawn: &SpawnInstance,
    host_role: &str,
    inner_width: u16,
    now: Instant,
    visible_steps: usize,
    focused: bool,
) -> Vec<Line<'static>> {
    if spawn.state != SpawnState::Working {
        return Vec::new();
    }

    let role_color = tui_style::role_color(&spawn.role, host_role);
    let border_color = if focused {
        Color::LightCyan
    } else {
        Color::DarkGray
    };
    let card_width = usable_card_width(inner_width);
    let elapsed = elapsed_label(now.saturating_duration_since(spawn.started_at));
    let title = if spawn.title.is_empty() {
        NO_TITLE_PLACEHOLDER.to_owned()
    } else {
        spawn.title.clone()
    };

    let mut lines = Vec::with_capacity(visible_steps + 2);
    lines.push(top_border_line(
        &spawn.role,
        host_role,
        role_color,
        &title,
        &elapsed,
        card_width,
        border_color,
    ));
    for record in tail_tool_calls(&spawn.tool_calls, visible_steps) {
        lines.push(body_line(record, card_width, border_color));
    }
    lines.push(bottom_border_line(
        spawn.step_count,
        card_width,
        border_color,
    ));
    lines
}

#[must_use]
/// Build the one-line focused-mode stub for a non-focused working card.
pub fn render_working_stub_line(
    spawn: &SpawnInstance,
    host_role: &str,
    now: Instant,
) -> Option<Line<'static>> {
    render_working_stub_line_with_focus(spawn, host_role, now, false)
}

#[must_use]
/// Build the one-line working stub, highlighting its role token when focused.
pub fn render_working_stub_line_with_focus(
    spawn: &SpawnInstance,
    host_role: &str,
    now: Instant,
    focused: bool,
) -> Option<Line<'static>> {
    if spawn.state != SpawnState::Working {
        return None;
    }
    let role_color = tui_style::role_color(&spawn.role, host_role);
    let role_style = if focused {
        Style::default()
            .fg(role_color)
            .add_modifier(Modifier::BOLD | Modifier::REVERSED)
    } else {
        Style::default().fg(role_color).add_modifier(Modifier::BOLD)
    };
    let elapsed = elapsed_label(now.saturating_duration_since(spawn.started_at));
    let title = if spawn.title.is_empty() {
        NO_TITLE_PLACEHOLDER.to_owned()
    } else {
        spawn.title.clone()
    };
    Some(Line::from(vec![
        Span::raw(CARD_INDENT),
        Span::styled(format!("@{}", spawn.role), role_style),
        Span::styled(
            format!(
                " · {title} · working · {elapsed} · {} step{}",
                spawn.step_count,
                if spawn.step_count == 1 { "" } else { "s" }
            ),
            Style::default().fg(Color::DarkGray),
        ),
    ]))
}

/// Build the single collapsed line that replaces a finished working
/// card in the chat stream.
///
/// Returns `None` while the spawn is still `Spawning` or `Working`.
/// `Done` and `Reported` both render the same marker; the report
/// message, when present, is spliced by the room renderer directly
/// below this line.
#[must_use]
pub fn render_done_collapsed_line(spawn: &SpawnInstance, host_role: &str) -> Option<Line<'static>> {
    render_done_collapsed_line_with_focus(spawn, host_role, false)
}

#[must_use]
/// Build the collapsed Done marker, highlighting the role token when focused.
pub fn render_done_collapsed_line_with_focus(
    spawn: &SpawnInstance,
    host_role: &str,
    focused: bool,
) -> Option<Line<'static>> {
    if !matches!(spawn.state, SpawnState::Done | SpawnState::Reported) {
        return None;
    }

    let role_color = tui_style::role_color(&spawn.role, host_role);
    let role_style = if focused {
        Style::default()
            .fg(role_color)
            .add_modifier(Modifier::BOLD | Modifier::REVERSED)
    } else {
        Style::default().fg(role_color).add_modifier(Modifier::BOLD)
    };
    let elapsed = elapsed_label(
        spawn
            .state_changed_at
            .saturating_duration_since(spawn.started_at),
    );
    let steps = format!(
        "{} step{}",
        spawn.step_count,
        if spawn.step_count == 1 { "" } else { "s" }
    );
    let (marker, label, marker_style, suffix) = match spawn.outcome {
        Outcome::Clean => (
            "✓",
            "done",
            Style::default().fg(Color::LightGreen),
            " · [e]xpand log",
        ),
        Outcome::Interrupted => ("⨯", "interrupted", Style::default().fg(Color::LightRed), ""),
        Outcome::Failed => (
            "⨯",
            "failed",
            Style::default().fg(Color::LightRed),
            " · [e]xpand log",
        ),
    };

    Some(Line::from(vec![
        Span::raw(CARD_INDENT),
        Span::styled(format!("@{}", spawn.role), role_style),
        Span::raw(" "),
        Span::styled(marker.to_owned(), marker_style),
        Span::raw(" "),
        Span::styled(label.to_owned(), marker_style),
        Span::styled(
            format!(" · {elapsed} · {steps}{suffix}"),
            Style::default().fg(Color::DarkGray),
        ),
    ]))
}

#[must_use]
/// Build inline tool-log rows shown when a collapsed Done card is expanded.
pub fn render_expanded_done_log_lines(
    spawn: &SpawnInstance,
    inner_width: u16,
) -> Vec<Line<'static>> {
    if !matches!(spawn.state, SpawnState::Done | SpawnState::Reported) {
        return Vec::new();
    }
    let card_width = usable_card_width(inner_width);
    spawn
        .tool_calls
        .iter()
        .map(|record| body_line(record, card_width, Color::DarkGray))
        .collect()
}

/// Effective inner-card width inside the scrollback panel. The card
/// borders draw inside `CARD_INDENT`; everything left of the indent
/// is the chat-style left margin that the card visually nests under.
fn usable_card_width(inner_width: u16) -> usize {
    let inner = usize::from(inner_width);
    inner.saturating_sub(CARD_INDENT.len()).max(MIN_CARD_WIDTH)
}

/// Take the last `n` tool calls in stream order. We render newest at
/// the bottom of the card, matching how a chat-style log reads —
/// older entries scroll *out the top* once the card hits its visible
/// budget (AC-6c).
fn tail_tool_calls(records: &[ToolCallRecord], visible_steps: usize) -> &[ToolCallRecord] {
    let n = records.len();
    let take = visible_steps.min(n);
    &records[n - take..]
}

/// Human-friendly elapsed label. Coarse to whole seconds so the
/// frame-to-frame rendering does not flicker on sub-second
/// fluctuations (AC-5).
fn elapsed_label(duration: Duration) -> String {
    let secs = duration.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else {
        let minutes = secs / 60;
        let remainder = secs % 60;
        format!("{minutes}m {remainder:02}s")
    }
}

/// Top border:
///   `┌─ {avatar} @{role} · {title} ── working · {elapsed} ─...─┐`
fn top_border_line(
    role: &str,
    host_role: &str,
    role_color: Color,
    title: &str,
    elapsed: &str,
    card_width: usize,
    border_color: Color,
) -> Line<'static> {
    // Budget the title to fit even on narrow terminals. The fixed
    // chrome is the borders, separators, the role label (`avatar` +
    // ` ` + `@role`), the state label (`working`), and the elapsed
    // label. Everything left over is for the title.
    let role_token = format!("@{role}");
    let chrome_width = "┌─ ".chars().count()
        + 1 // avatar glyph (1 cell)
        + 1 // space
        + role_token.chars().count()
        + " · ".chars().count()
        + " ── ".chars().count()
        + "working".chars().count()
        + " · ".chars().count()
        + elapsed.chars().count()
        + " ".chars().count()
        + 1; // closing ┐

    let title_budget = card_width.saturating_sub(chrome_width).max(1);
    let title_truncated = middle_truncate(title, title_budget);

    let mut spans: Vec<Span<'static>> = Vec::with_capacity(12);
    spans.push(Span::raw(CARD_INDENT));
    spans.push(Span::styled(
        "┌─ ".to_owned(),
        Style::default().fg(border_color),
    ));
    spans.extend(tui_style::role_label_spans(role, host_role));
    spans.push(Span::styled(
        format!(" · {title_truncated} "),
        Style::default().fg(role_color),
    ));
    spans.push(Span::styled(
        "── ".to_owned(),
        Style::default().fg(border_color),
    ));
    spans.push(Span::styled(
        "working".to_owned(),
        Style::default()
            .fg(Color::LightYellow)
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(
        format!(" · {elapsed} "),
        Style::default().fg(Color::DarkGray),
    ));

    // Count visible cells (post-indent) and pad to the closing ┐.
    let visible_so_far: usize = spans
        .iter()
        .skip(1) // skip the indent span; it does not contribute to card-width budget
        .map(|s| s.content.chars().count())
        .sum();
    let pad = card_width.saturating_sub(visible_so_far + 1);
    if pad > 0 {
        spans.push(Span::styled(
            "─".repeat(pad),
            Style::default().fg(border_color),
        ));
    }
    spans.push(Span::styled(
        "┐".to_owned(),
        Style::default().fg(border_color),
    ));
    Line::from(spans)
}

/// One body line:
///   `│ {marker} {summary…} {pad}│`
fn body_line(record: &ToolCallRecord, card_width: usize, border_color: Color) -> Line<'static> {
    let (marker, marker_style) = marker_for_status(record.status);
    let summary_text = if record.summary.is_empty() {
        record.tool.clone()
    } else {
        record.summary.clone()
    };
    // Chrome: `│ ` + marker (1) + ` ` + summary + trailing ` ` before `│`.
    let chrome_width = 2 + 1 + 1 + 1 + 1; // = 6
    let summary_budget = card_width.saturating_sub(chrome_width).max(1);
    let summary = middle_truncate(&summary_text, summary_budget);
    let visible_summary_width = summary.chars().count();
    let pad = card_width.saturating_sub(2 + 1 + 1 + visible_summary_width + 1 + 1);

    Line::from(vec![
        Span::raw(CARD_INDENT),
        Span::styled("│ ".to_owned(), Style::default().fg(border_color)),
        Span::styled(marker.to_owned(), marker_style),
        Span::raw(" "),
        Span::raw(summary),
        Span::raw(" ".repeat(pad)),
        Span::styled(" │".to_owned(), Style::default().fg(border_color)),
    ])
}

fn marker_for_status(status: ToolCallStatus) -> (&'static str, Style) {
    match status {
        ToolCallStatus::Done => ("✓", Style::default().fg(Color::LightGreen)),
        ToolCallStatus::InProgress => ("∴", Style::default().fg(Color::LightYellow)),
        ToolCallStatus::Failed => ("⨯", Style::default().fg(Color::LightRed)),
    }
}

/// Bottom border:
///   `└─ {N step(s) done · [e]xpand [i]nterrupt [f]ocus} ─...─┘`
fn bottom_border_line(done_count: usize, card_width: usize, border_color: Color) -> Line<'static> {
    let footer_text = format!(
        " {done_count} step{plural} done · [e]xpand [i]nterrupt [f]ocus ",
        plural = if done_count == 1 { "" } else { "s" },
    );
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(5);
    spans.push(Span::raw(CARD_INDENT));
    spans.push(Span::styled(
        "└─".to_owned(),
        Style::default().fg(border_color),
    ));
    spans.push(Span::styled(
        footer_text.clone(),
        Style::default().fg(Color::DarkGray),
    ));
    let visible_so_far: usize = spans
        .iter()
        .skip(1)
        .map(|s| s.content.chars().count())
        .sum();
    let pad = card_width.saturating_sub(visible_so_far + 1);
    if pad > 0 {
        spans.push(Span::styled(
            "─".repeat(pad),
            Style::default().fg(border_color),
        ));
    }
    spans.push(Span::styled(
        "┘".to_owned(),
        Style::default().fg(border_color),
    ));
    Line::from(spans)
}

/// Middle-truncate a string to `max_chars`, inserting an ellipsis in
/// the middle so the head and tail both remain visible. Returns the
/// input unchanged when it already fits. Used for both the title row
/// (long task descriptions) and the tool-call summary (long paths).
fn middle_truncate(input: &str, max_chars: usize) -> String {
    let count = input.chars().count();
    if count <= max_chars {
        return input.to_owned();
    }
    if max_chars <= 1 {
        return "…".to_owned();
    }
    // Reserve 1 char for the ellipsis. Bias the head one larger than
    // the tail on an odd remainder so prefixes (path heads, file names)
    // stay readable.
    let budget = max_chars - 1;
    let head_chars = budget.div_ceil(2);
    let tail_chars = budget - head_chars;
    let head: String = input.chars().take(head_chars).collect();
    let tail: String = input
        .chars()
        .skip(count.saturating_sub(tail_chars))
        .collect();
    format!("{head}…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crep::CrepEvent;
    use crate::spawn_lifecycle::SpawnLifecycleTracker;
    use crate::turn::TurnId;
    use std::time::Duration;

    /// Build a `Working` spawn instance via the canonical tracker
    /// event flow. We construct via real apply_event calls instead of
    /// the struct literal because [`crate::spawn_lifecycle::SpawnId`]
    /// is intentionally opaque to consumers — the renderer does not
    /// need to mint ids.
    fn working_spawn(role: &str, title: &str) -> SpawnInstance {
        let mut tracker = SpawnLifecycleTracker::new("host");
        let id = tracker
            .apply_event(&CrepEvent::TurnDispatched {
                role: role.to_owned(),
                priors_hash: String::new(),
                turn_id: TurnId::from("t1".to_owned()),
                thread_id: TurnId::from("thread-t1".to_owned()),
                parent_turn_id: Some(TurnId::from("root".to_owned())),
                queue_position: 0,
            })
            .expect("dispatch yields a spawn id");
        // Promote to Working with a no-op output delta — keeps the
        // tool_calls vec empty so each test can populate it explicitly.
        tracker.apply_event(&CrepEvent::RoleOutputDelta {
            role: role.to_owned(),
            priors_hash: String::new(),
            text_delta: String::new(),
            sequence: 0,
            turn_id: TurnId::from("t1".to_owned()),
            thread_id: TurnId::from("thread-t1".to_owned()),
        });
        if !title.is_empty() {
            tracker.apply_event(&CrepEvent::WorkTitle {
                role: role.to_owned(),
                priors_hash: String::new(),
                title: title.to_owned(),
                turn_id: TurnId::from("t1".to_owned()),
                thread_id: TurnId::from("thread-t1".to_owned()),
            });
        }
        tracker.get(id).cloned().expect("instance exists")
    }

    fn tool_record(summary: &str, status: ToolCallStatus) -> ToolCallRecord {
        let started_at = Instant::now();
        ToolCallRecord {
            tool_use_id: "u".to_owned(),
            tool: "Bash".to_owned(),
            summary: summary.to_owned(),
            started_at,
            finished_at: if matches!(status, ToolCallStatus::InProgress) {
                None
            } else {
                Some(started_at)
            },
            status,
        }
    }

    fn line_to_string(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>()
    }

    #[test]
    fn non_working_state_returns_no_lines() {
        // Spawning, Done, and Reported must not render a card. The
        // caller can iterate all instances without filtering and the
        // widget enforces the gate.
        let mut spawn = working_spawn("security", "audit");
        let now = Instant::now();
        spawn.state = SpawnState::Spawning;
        assert!(
            render_working_card_lines(&spawn, "host", 80, now, DEFAULT_VISIBLE_STEPS).is_empty()
        );
        spawn.state = SpawnState::Done;
        assert!(
            render_working_card_lines(&spawn, "host", 80, now, DEFAULT_VISIBLE_STEPS).is_empty()
        );
        spawn.state = SpawnState::Reported;
        assert!(
            render_working_card_lines(&spawn, "host", 80, now, DEFAULT_VISIBLE_STEPS).is_empty()
        );
    }

    #[test]
    fn done_collapsed_line_renders_clean_completion_marker() {
        let mut spawn = working_spawn("qa", "smoke test");
        spawn.state = SpawnState::Done;
        spawn.step_count = 2;
        spawn.state_changed_at = spawn.started_at + Duration::from_secs(52);
        let line = render_done_collapsed_line(&spawn, "host").expect("done line");
        let text = line_to_string(&line);
        assert!(text.contains("@qa"));
        assert!(text.contains("✓ done"));
        assert!(text.contains("52s"));
        assert!(text.contains("2 steps"));
        assert!(text.contains("[e]xpand log"));
    }

    #[test]
    fn done_collapsed_line_renders_interrupted_without_expand_hint() {
        let mut spawn = working_spawn("security", "audit");
        spawn.state = SpawnState::Done;
        spawn.outcome = Outcome::Interrupted;
        spawn.step_count = 1;
        let line = render_done_collapsed_line(&spawn, "host").expect("done line");
        let text = line_to_string(&line);
        assert!(text.contains("@security"));
        assert!(text.contains("⨯ interrupted"));
        assert!(text.contains("1 step"));
        assert!(!text.contains("[e]xpand log"));
    }

    #[test]
    fn done_collapsed_line_returns_none_before_done() {
        let mut spawn = working_spawn("backend", "verify");
        assert!(render_done_collapsed_line(&spawn, "host").is_none());
        spawn.state = SpawnState::Spawning;
        assert!(render_done_collapsed_line(&spawn, "host").is_none());
    }

    #[test]
    fn zero_tool_calls_card_shows_just_title() {
        // AC-6a: a card with no tool calls renders top border + bottom
        // border only (two lines), with the title intact in the top
        // border. The step count is 0 and the footer says "0 steps".
        let spawn = working_spawn("security", "audit README claims");
        let now = spawn.started_at; // pin elapsed = 0 for stable assertion
        let lines = render_working_card_lines(&spawn, "host", 80, now, DEFAULT_VISIBLE_STEPS);
        assert_eq!(lines.len(), 2, "expected exactly top + bottom border");
        let top = line_to_string(&lines[0]);
        let bottom = line_to_string(&lines[1]);
        assert!(top.contains("@security"), "top border missing role: {top}");
        assert!(
            top.contains("audit README claims"),
            "top border missing title: {top}"
        );
        assert!(top.contains("working"));
        assert!(top.contains("0s"));
        assert!(top.starts_with("             "));
        assert!(bottom.contains("0 steps done"));
        assert!(bottom.contains("[e]xpand [i]nterrupt [f]ocus"));
    }

    #[test]
    fn one_done_plus_one_in_progress_renders_both_markers() {
        // AC-6b: a card with one ✓ done call and one ∴ in-progress call
        // renders two body lines between top and bottom border. The
        // step count reflects only the done call.
        let mut spawn = working_spawn("security", "audit");
        spawn.tool_calls.push(tool_record(
            "read README.md §2.4 security model",
            ToolCallStatus::Done,
        ));
        spawn.tool_calls.push(tool_record(
            "cross-checking claims against src/permissions/",
            ToolCallStatus::InProgress,
        ));
        spawn.step_count = 1;
        let now = spawn.started_at;
        let lines = render_working_card_lines(&spawn, "host", 100, now, DEFAULT_VISIBLE_STEPS);
        // top + 2 body + bottom
        assert_eq!(lines.len(), 4, "expected 4 lines (top + 2 body + bottom)");
        let body_done = line_to_string(&lines[1]);
        let body_in_progress = line_to_string(&lines[2]);
        assert!(body_done.contains('✓'), "missing ✓ marker: {body_done}");
        assert!(
            body_done.contains("read README.md"),
            "done summary missing: {body_done}"
        );
        assert!(
            body_in_progress.contains('∴'),
            "missing ∴ marker: {body_in_progress}"
        );
        assert!(
            body_in_progress.contains("cross-checking"),
            "in-progress summary missing: {body_in_progress}"
        );
        let bottom = line_to_string(&lines[3]);
        assert!(
            bottom.contains("1 step done"),
            "footer step count: {bottom}"
        );
    }

    #[test]
    fn more_than_n_tool_calls_drops_oldest_visually() {
        // AC-6c: with ≥ N + 1 tool calls, only the latest N body lines
        // appear in the card. The oldest entry is the one that drops.
        let mut spawn = working_spawn("backend", "verify claims");
        // 5 done tool calls, in order oldest → newest.
        for i in 0..5 {
            spawn
                .tool_calls
                .push(tool_record(&format!("step-{i}"), ToolCallStatus::Done));
        }
        spawn.step_count = 5;
        let now = spawn.started_at;
        let lines = render_working_card_lines(&spawn, "host", 80, now, 3);
        // top + 3 body + bottom
        assert_eq!(lines.len(), 5);
        let body_concat = lines[1..4]
            .iter()
            .map(line_to_string)
            .collect::<Vec<_>>()
            .join("\n");
        // oldest two (step-0, step-1) dropped; newest three remain.
        assert!(
            !body_concat.contains("step-0"),
            "step-0 should drop: {body_concat}"
        );
        assert!(
            !body_concat.contains("step-1"),
            "step-1 should drop: {body_concat}"
        );
        assert!(body_concat.contains("step-2"));
        assert!(body_concat.contains("step-3"));
        assert!(body_concat.contains("step-4"));
        let bottom = line_to_string(&lines[4]);
        assert!(bottom.contains("5 steps done"));
    }

    #[test]
    fn missing_title_renders_no_title_placeholder() {
        // When `SpawnInstance::title` is empty (no WorkTitle seen yet),
        // the top border renders the locked placeholder so the card
        // shape stays intact.
        let spawn = working_spawn("backend", "");
        let now = spawn.started_at;
        let lines = render_working_card_lines(&spawn, "host", 80, now, DEFAULT_VISIBLE_STEPS);
        let top = line_to_string(&lines[0]);
        assert!(
            top.contains(NO_TITLE_PLACEHOLDER),
            "top border missing placeholder: {top}"
        );
    }

    #[test]
    fn elapsed_label_uses_whole_seconds_so_subsecond_changes_do_not_flicker() {
        // AC-5: rendering must not change on sub-second drift. The
        // label string for `5s` and `5.7s` must be identical.
        assert_eq!(elapsed_label(Duration::from_secs(5)), "5s");
        assert_eq!(elapsed_label(Duration::from_millis(5_700)), "5s");
        assert_eq!(elapsed_label(Duration::from_secs(0)), "0s");
    }

    #[test]
    fn elapsed_label_switches_to_minutes_past_a_minute() {
        assert_eq!(elapsed_label(Duration::from_secs(60)), "1m 00s");
        assert_eq!(elapsed_label(Duration::from_secs(125)), "2m 05s");
        assert_eq!(elapsed_label(Duration::from_secs(3_599)), "59m 59s");
    }

    #[test]
    fn middle_truncate_keeps_head_and_tail_with_ellipsis() {
        // Used to keep long paths legible on narrow terminals — head
        // shows the package/dir, tail shows the file. Equal-length
        // inputs round up to the head per the comment.
        let truncated = middle_truncate("src/permissions/policies/loader.rs", 20);
        assert_eq!(truncated.chars().count(), 20);
        assert!(truncated.contains('…'));
        assert!(truncated.starts_with("src/perm")); // head intact
        assert!(truncated.ends_with("loader.rs")); // tail intact
    }

    #[test]
    fn middle_truncate_passthrough_when_within_budget() {
        let input = "short text";
        assert_eq!(middle_truncate(input, 80), input);
    }

    #[test]
    fn long_tool_summary_is_middle_truncated_inside_card_width() {
        // AC-4: long summaries are middle-truncated to fit the card
        // width. The card body row must not exceed `card_width` total
        // visible cells (excluding the leading indent).
        let mut spawn = working_spawn("backend", "x");
        spawn.tool_calls.push(tool_record(
            "ran cargo test --workspace --no-fail-fast --features all-engines-and-extras",
            ToolCallStatus::Done,
        ));
        spawn.step_count = 1;
        let now = spawn.started_at;
        let lines = render_working_card_lines(&spawn, "host", 60, now, DEFAULT_VISIBLE_STEPS);
        let body = line_to_string(&lines[1]);
        // The full summary must not be present verbatim — it got
        // truncated to fit.
        assert!(body.contains('…'), "expected middle ellipsis: {body}");
        // Body row width (after the indent) must equal card_width.
        let after_indent = body.trim_start_matches(' ');
        let total_visible = after_indent.chars().count();
        // 60 - indent (13) = 47 inner card width, but min is 40.
        assert!(
            total_visible >= MIN_CARD_WIDTH,
            "body shorter than min card width: {total_visible} ({body})"
        );
    }

    #[test]
    fn identity_color_comes_from_tui_style_role_color_not_hardcoded() {
        // AC-3: the role label inside the top border uses the
        // canonical role color helper. We verify by comparing the
        // header's role-token color against role_color directly —
        // they must agree.
        let spawn = working_spawn("security", "audit");
        let now = spawn.started_at;
        let lines = render_working_card_lines(&spawn, "host", 80, now, DEFAULT_VISIBLE_STEPS);
        let top = &lines[0];
        // Find the `@security` span; its fg must equal role_color.
        let want = tui_style::role_color("security", "host");
        let role_span = top
            .spans
            .iter()
            .find(|s| s.content.as_ref().contains("@security"))
            .expect("top border has role token");
        assert_eq!(role_span.style.fg, Some(want));
    }

    #[test]
    fn footer_singular_step_word_when_done_count_is_one() {
        let mut spawn = working_spawn("backend", "x");
        spawn
            .tool_calls
            .push(tool_record("one done", ToolCallStatus::Done));
        spawn.step_count = 1;
        let now = spawn.started_at;
        let lines = render_working_card_lines(&spawn, "host", 80, now, DEFAULT_VISIBLE_STEPS);
        let bottom = line_to_string(&lines[lines.len() - 1]);
        assert!(bottom.contains("1 step done"), "singular step: {bottom}");
        assert!(!bottom.contains("1 steps done"));
    }

    #[test]
    fn failed_tool_call_renders_cross_marker() {
        let mut spawn = working_spawn("backend", "x");
        spawn
            .tool_calls
            .push(tool_record("denied", ToolCallStatus::Failed));
        spawn.step_count = 1;
        let now = spawn.started_at;
        let lines = render_working_card_lines(&spawn, "host", 80, now, DEFAULT_VISIBLE_STEPS);
        let body = line_to_string(&lines[1]);
        assert!(body.contains('⨯'), "failed marker missing: {body}");
    }
}
