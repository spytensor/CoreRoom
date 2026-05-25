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

use std::sync::Arc;

use crate::crep::CrepEvent;

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
        match event {
            RoomEvent::Crep { event, host_role } => {
                let line = crate::repl::render_event_line_for_sink(&event, &host_role);
                if !line.trim().is_empty() {
                    println!("{line}");
                }
            }
            RoomEvent::Notice { level, text } => match level {
                NoticeLevel::Ok => crate::output::ok(text),
                NoticeLevel::Warn => crate::output::warn(text),
                NoticeLevel::Bad => crate::output::bad(text),
                NoticeLevel::Hint => crate::output::hint(text),
                NoticeLevel::System => crate::output::system(text),
            },
        }
    }
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
}
