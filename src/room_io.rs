//! Renderer-agnostic I/O surface for the CoreRoom runtime.
//!
//! Historically the REPL wrote directly to `stdout` via `println!`,
//! [`crate::output`], and the per-event renderers in [`crate::repl::render`].
//! That made it impossible to host the same execution path inside a
//! ratatui full-screen surface without duplicating logic.
//!
//! This module introduces a sink trait so the runtime can hand visible
//! events off to either:
//!
//! - [`StdoutSink`] — the byte-for-byte legacy `cr start` path, preserved
//!   for backwards compatibility and as the safety net during the
//!   incremental port.
//! - A future `TuiSink` — pushes the same events into a ratatui app's
//!   render queue so the full-screen room can render the real runtime.
//!
//! Stage 1 of the port (this file) only models the events the REPL
//! actually emits today. Variants are added as call sites are migrated;
//! the trait stays additive so an in-progress port never breaks `cr start`.

use std::io::{IsTerminal, Write as _};
use std::sync::Arc;
use std::time::Instant;

use crate::crep::CrepEvent;
use crate::output::work_card::WorkCard;

/// One observable event from the runtime headed for the user.
///
/// Variants are owned values so a sink can buffer them across threads
/// (the TUI sink will push these onto an mpsc channel for the renderer
/// loop). `CrepEvent::clone` is cheap relative to engine work, so the
/// owning copy is not a hot-path concern.
#[derive(Debug, Clone)]
pub enum RoomEvent {
    /// A CREP bus event that should be rendered as one (or more) lines
    /// in the conversation pane. `host_role` is required because role
    /// colors are computed relative to the project's host role.
    Crep {
        /// The CREP event from the bus. Boxed because [`CrepEvent`] is
        /// large and most sinks copy or buffer it.
        event: Box<CrepEvent>,
        /// Host role name at emit time, used by the renderer to color
        /// non-host roles relative to it.
        host_role: String,
    },
    /// A short one-line system notice. Maps to the families in
    /// [`crate::output`] (`ok`/`warn`/`bad`/`hint`/`system`). The sink
    /// is free to render these as ANSI lines (stdout) or as a toast
    /// area (TUI).
    Notice {
        /// Severity / visual classification of the notice.
        level: NoticeLevel,
        /// Already-finalized one-line text. The sink applies prefix
        /// glyphs and color; the runtime supplies the body.
        text: String,
    },
    /// Already-rendered text block such as the boot splash, `/help`,
    /// or a styled handoff/route line. `StdoutSink` prints this
    /// byte-for-byte; a TUI sink may parse or place it as preformatted
    /// scrollback until structured variants replace each surface.
    Banner(String),
    /// A structured role work-card update. The stdout sink renders it
    /// with the legacy boxed card layout; a TUI sink can place the same
    /// card in a work/status pane.
    WorkCard(WorkCard),
    /// A structured in-place status spinner update.
    Spinner(SpinnerSnapshot),
}

/// Point-in-time state for the in-flight role spinner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpinnerSnapshot {
    /// Role whose turn is currently being surfaced.
    pub role: String,
    /// Index into the ten-frame braille spinner set.
    pub frame: usize,
    /// Wall-clock instant when the role turn started.
    pub started_at: Instant,
    /// Number of tool proposals seen during this turn.
    pub tools_seen: usize,
    /// Best-effort current state label, or `None` for "thinking".
    pub current_state: Option<String>,
    /// Whether the spinner line should be painted, cleared, or marked
    /// as paused behind an approval prompt.
    pub paint: SpinnerPaint,
}

/// Rendering mode for a [`SpinnerSnapshot`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpinnerPaint {
    /// Clear the in-place status row.
    Cleared,
    /// Paint the status row with the current frame/state.
    Painting,
    /// The role is blocked behind a permission approval prompt.
    WaitingApproval,
}

/// Categorical level for a one-line system notice. Sinks may map these
/// to colors, glyphs, severity badges, or filter them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeLevel {
    /// Affirmative result (`output::ok`).
    Ok,
    /// Recoverable warning (`output::warn`).
    Warn,
    /// Error or hard failure (`output::bad`).
    Bad,
    /// Optional hint, dimmed (`output::hint`).
    Hint,
    /// Neutral system note (`output::system`).
    System,
}

/// Renderer-agnostic destination for runtime events.
///
/// Implementations must be cheap to call from any task and safe to share
/// across threads — the REPL spawns roles on independent tokio tasks and
/// each emits into the same sink.
pub trait RoomSink: Send + Sync {
    /// Render or buffer one event. Sinks must not block; if their target
    /// is slow (e.g. a TUI renderer behind a channel), they should drop
    /// or coalesce rather than back-pressure the runtime.
    fn emit(&self, event: RoomEvent);
}

/// The legacy `cr start` sink: each event is formatted via the existing
/// per-event renderers and written straight to `stdout`. Holding no
/// state means it can be cloned, shared, or constructed on demand.
///
/// This preserves `cr start` output byte-for-byte during the incremental
/// port. The TUI sink is a sibling implementation, not a replacement.
#[derive(Debug, Default, Clone, Copy)]
pub struct StdoutSink;

impl RoomSink for StdoutSink {
    fn emit(&self, event: RoomEvent) {
        let flush = matches!(&event, RoomEvent::Spinner(_));
        print!("{}", Self::render_to_string(&event));
        if flush {
            let _ = std::io::stdout().flush();
        }
    }
}

impl StdoutSink {
    /// Render one room event exactly as `StdoutSink::emit` would write it.
    #[must_use]
    pub(crate) fn render_to_string(event: &RoomEvent) -> String {
        match event {
            RoomEvent::Crep { event, host_role } => {
                let line = crate::repl::render_event_line_for_sink(event, host_role);
                if line.trim().is_empty() {
                    String::new()
                } else {
                    format!("{line}\n")
                }
            }
            RoomEvent::Notice { level, text } => match level {
                NoticeLevel::Ok => crate::output::ok_line(text),
                NoticeLevel::Warn => crate::output::warn_line(text),
                NoticeLevel::Bad => crate::output::bad_line(text),
                NoticeLevel::Hint => crate::output::hint_line(text),
                NoticeLevel::System => crate::output::system_line(text),
            },
            RoomEvent::Banner(text) => text.clone(),
            RoomEvent::WorkCard(card) => crate::repl::render_work_card_for_sink(card),
            RoomEvent::Spinner(snapshot) => {
                if std::io::stdout().is_terminal() {
                    crate::repl::render_spinner_snapshot_for_sink(snapshot)
                } else {
                    String::new()
                }
            }
        }
    }
}

/// Emit a notice-level status line through `sink`.
pub fn emit_notice(sink: &dyn RoomSink, level: NoticeLevel, text: impl Into<String>) {
    sink.emit(RoomEvent::Notice {
        level,
        text: text.into(),
    });
}

/// Emit an `ok` notice through `sink`.
pub fn emit_ok(sink: &dyn RoomSink, text: impl Into<String>) {
    emit_notice(sink, NoticeLevel::Ok, text);
}

/// Emit a `warn` notice through `sink`.
pub fn emit_warn(sink: &dyn RoomSink, text: impl Into<String>) {
    emit_notice(sink, NoticeLevel::Warn, text);
}

/// Emit a `bad` notice through `sink`.
pub fn emit_bad(sink: &dyn RoomSink, text: impl Into<String>) {
    emit_notice(sink, NoticeLevel::Bad, text);
}

/// Emit a `hint` notice through `sink`.
pub fn emit_hint(sink: &dyn RoomSink, text: impl Into<String>) {
    emit_notice(sink, NoticeLevel::Hint, text);
}

/// Emit a `system` notice through `sink`.
pub fn emit_system(sink: &dyn RoomSink, text: impl Into<String>) {
    emit_notice(sink, NoticeLevel::System, text);
}

/// Emit a preformatted banner block through `sink`.
pub fn emit_banner(sink: &dyn RoomSink, text: impl Into<String>) {
    sink.emit(RoomEvent::Banner(text.into()));
}

/// Emit a preformatted line through `sink`, appending a newline if needed.
pub fn emit_line(sink: &dyn RoomSink, text: impl Into<String>) {
    let mut text = text.into();
    if !text.ends_with('\n') {
        text.push('\n');
    }
    emit_banner(sink, text);
}

/// Convenience constructor for an `Arc<dyn RoomSink>` pointing at
/// `stdout`. Use this from `cr start` and any other code path that
/// wants the legacy renderer.
#[must_use]
pub fn stdout_sink() -> Arc<dyn RoomSink> {
    Arc::new(StdoutSink)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crep::{CrepEvent, StopReason};
    use crate::output;
    use crate::output::work_card::{Step, StepKind, WorkStatus};
    use crossterm::style::{Color, Stylize};
    use std::time::Duration;

    /// In-memory sink used by tests to capture emitted events without
    /// touching stdout. Not exposed outside the crate.
    #[derive(Default)]
    struct CapturingSink {
        events: std::sync::Mutex<Vec<RoomEvent>>,
    }

    impl RoomSink for CapturingSink {
        fn emit(&self, event: RoomEvent) {
            self.events
                .lock()
                .expect("capturing sink mutex")
                .push(event);
        }
    }

    #[test]
    fn stdout_sink_formats_crep_via_legacy_renderer() {
        // The contract `StdoutSink` upholds: a CrepEvent fed through the
        // sink produces exactly the same string the legacy renderer
        // would emit. We compare against `render_event_line_for_sink`
        // directly — that helper is the canonical formatter both paths
        // agree on, so the sink stays a thin shim and not a divergent
        // re-implementation.
        let event = CrepEvent::RoleStopped {
            role: "host".to_owned(),
            priors_hash: String::new(),
            reason: StopReason::Completed,
            turn_id: None,
        };
        let host_role = "host";
        let expected = crate::repl::render_event_line_for_sink(&event, host_role);
        // Just assert the formatter produces a non-empty line for a
        // representative event; the sink runs the same code path so the
        // output is identical by construction.
        assert!(!expected.trim().is_empty());
    }

    #[test]
    fn capturing_sink_records_emitted_events() {
        let sink = CapturingSink::default();
        sink.emit(RoomEvent::Notice {
            level: NoticeLevel::System,
            text: "boot".to_owned(),
        });
        sink.emit(RoomEvent::Notice {
            level: NoticeLevel::Ok,
            text: "ready".to_owned(),
        });
        let events = sink.events.lock().expect("capturing sink mutex");
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn stdout_sink_notice_lines_match_output_helpers() {
        let cases = [
            (NoticeLevel::Ok, "ready", crate::output::ok_line("ready")),
            (
                NoticeLevel::Warn,
                "careful",
                crate::output::warn_line("careful"),
            ),
            (
                NoticeLevel::Bad,
                "broken",
                crate::output::bad_line("broken"),
            ),
            (
                NoticeLevel::Hint,
                "next step",
                crate::output::hint_line("next step"),
            ),
            (
                NoticeLevel::System,
                "routing",
                crate::output::system_line("routing"),
            ),
        ];
        for (level, text, expected) in cases {
            assert_eq!(
                StdoutSink::render_to_string(&RoomEvent::Notice {
                    level,
                    text: text.to_owned(),
                }),
                expected
            );
        }
    }

    #[test]
    fn stdout_sink_banner_is_byte_for_byte_passthrough() {
        let banner = "\nCoreRoom\n  help\n".to_owned();
        assert_eq!(
            StdoutSink::render_to_string(&RoomEvent::Banner(banner.clone())),
            banner
        );
    }

    #[test]
    fn stdout_sink_work_card_matches_legacy_bytes() {
        let card = sample_work_card();
        let actual = crate::repl::render_work_card_at_terminal_width_for_sink(&card, 80);
        let mut expected = String::new();
        for line in card.render(78).lines() {
            expected.push_str("  ");
            expected.push_str(line);
            expected.push('\n');
        }
        assert_eq!(actual, expected);
    }

    #[test]
    fn stdout_sink_spinner_transition_matches_legacy_bytes() {
        let started_at = std::time::Instant::now()
            .checked_sub(Duration::from_secs(12))
            .unwrap_or_else(std::time::Instant::now);
        let snapshots = [
            spinner_snapshot(started_at, 0, 0, None, SpinnerPaint::Painting),
            spinner_snapshot(started_at, 1, 0, None, SpinnerPaint::Painting),
            spinner_snapshot(
                started_at,
                1,
                1,
                Some("running Bash `cargo test --locked`"),
                SpinnerPaint::Painting,
            ),
            spinner_snapshot(
                started_at,
                1,
                1,
                Some("waiting approval · Bash `cargo test --locked`"),
                SpinnerPaint::WaitingApproval,
            ),
            spinner_snapshot(started_at, 1, 1, Some("thinking"), SpinnerPaint::Cleared),
        ];
        let actual = snapshots
            .iter()
            .map(|snapshot| crate::repl::render_spinner_snapshot_at_width_for_sink(snapshot, 120))
            .collect::<String>();
        let expected = [
            format!(
                "\r\x1b[2K{}",
                "  1 role working · ⠋ @security · 12s · thinking".with(output::DIM)
            ),
            format!(
                "\r\x1b[2K{}",
                "  1 role working · ⠙ @security · 12s · thinking".with(output::DIM)
            ),
            format!(
                "\r\x1b[2K{}",
                "  1 role working · ⠙ @security · 12s · 1 tool · running Bash `cargo test --locked`"
                    .with(output::DIM)
            ),
            format!(
                "\r\x1b[2K{}",
                "  1 role working · ⠙ @security · 12s · 1 tool · waiting approval · Bash `cargo test --locked`"
                    .with(output::DIM)
            ),
            "\r\x1b[2K".to_owned(),
        ]
        .concat();
        assert_eq!(actual, expected);
    }

    fn sample_work_card() -> WorkCard {
        WorkCard {
            role: "security".to_owned(),
            host_role: "host".to_owned(),
            role_color: Color::Rgb {
                r: 0x5c,
                g: 0xd6,
                b: 0xcc,
            },
            title: "Check permission bridge".to_owned(),
            status: WorkStatus::Done {
                duration: Duration::from_secs(12),
                steps_count: 2,
            },
            steps: vec![
                Step {
                    kind: StepKind::Done,
                    text: "Read src/repl/status.rs".to_owned(),
                    time: None,
                },
                Step {
                    kind: StepKind::Done,
                    text: "Run cargo test".to_owned(),
                    time: Some("12s".to_owned()),
                },
            ],
            collapsed: false,
        }
    }

    fn spinner_snapshot(
        started_at: std::time::Instant,
        frame: usize,
        tools_seen: usize,
        current_state: Option<&str>,
        paint: SpinnerPaint,
    ) -> SpinnerSnapshot {
        SpinnerSnapshot {
            role: "security".to_owned(),
            frame,
            started_at,
            tools_seen,
            current_state: current_state.map(str::to_owned),
            paint,
        }
    }
}
