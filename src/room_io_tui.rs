//! Event-driven room sink for the full-screen console runtime.
//!
//! `TuiSink` is deliberately small: the executable REPL owns behavior
//! and emits [`crate::room_io::RoomEvent`] values, while the ratatui
//! room owns presentation and user input.

use tokio::sync::mpsc;

use crate::room_io::{RoomEvent, RoomSink};

/// Non-blocking sink that forwards room events to a ratatui render loop.
#[derive(Debug, Clone)]
pub struct TuiSink {
    tx: mpsc::UnboundedSender<RoomEvent>,
}

impl TuiSink {
    /// Create a sink from an existing unbounded event sender.
    #[must_use]
    pub fn new(tx: mpsc::UnboundedSender<RoomEvent>) -> Self {
        Self { tx }
    }

    /// Create the sink and matching event receiver.
    #[must_use]
    pub fn channel() -> (Self, mpsc::UnboundedReceiver<RoomEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Self::new(tx), rx)
    }
}

impl RoomSink for TuiSink {
    fn emit(&self, event: RoomEvent) {
        // UnboundedSender::send is synchronous and never awaits. A closed
        // receiver means the TUI already exited, so dropping the event is
        // the only useful behavior.
        let _ = self.tx.send(event);
    }

    fn handles_permission_decisions(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crep::{CrepEvent, StopReason};
    use crate::output::work_card::{Step, StepKind, WorkCard, WorkStatus};
    use crate::permissions::{BridgeRequest, BridgeResponse, DecisionScope, PermissionDecision};
    use crate::room_io::{NoticeLevel, SpinnerPaint, SpinnerSnapshot};
    use crossterm::style::Color;
    use serde_json::json;
    use std::time::{Duration, Instant};

    #[tokio::test]
    async fn tui_sink_forwards_every_room_event_variant_without_blocking() {
        let (sink, mut rx) = TuiSink::channel();
        let events = representative_events();
        for event in events.clone() {
            sink.emit(event);
        }

        let mut received = Vec::new();
        while received.len() < events.len() {
            received.push(rx.recv().await.expect("event delivered"));
        }

        let expected = events
            .iter()
            .map(std::mem::discriminant)
            .collect::<Vec<_>>();
        let actual = received
            .iter()
            .map(std::mem::discriminant)
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }

    #[test]
    fn tui_sink_owns_permission_decisions() {
        let (sink, _rx) = TuiSink::channel();
        assert!(sink.handles_permission_decisions());
    }

    fn representative_events() -> Vec<RoomEvent> {
        let request = BridgeRequest {
            v: 1,
            role: "backend".to_owned(),
            tool: "Bash".to_owned(),
            input: json!({"command": "cargo test"}),
            reason: "ask".to_owned(),
        };
        vec![
            RoomEvent::Crep {
                event: Box::new(CrepEvent::RoleStopped {
                    role: "backend".to_owned(),
                    priors_hash: "abc".to_owned(),
                    reason: StopReason::Completed,
                    turn_id: None,
                }),
                host_role: "host".to_owned(),
            },
            RoomEvent::Notice {
                level: NoticeLevel::System,
                text: "boot".to_owned(),
            },
            RoomEvent::Banner("welcome\n".to_owned()),
            RoomEvent::WorkCard(sample_work_card()),
            RoomEvent::Spinner(SpinnerSnapshot {
                role: "backend".to_owned(),
                frame: 1,
                started_at: Instant::now()
                    .checked_sub(Duration::from_secs(2))
                    .unwrap_or_else(Instant::now),
                tools_seen: 1,
                current_state: Some("running Bash".to_owned()),
                paint: SpinnerPaint::Painting,
            }),
            RoomEvent::PermissionPrompt {
                request: request.clone(),
                host_role: "host".to_owned(),
                response_tx: None,
            },
            RoomEvent::PermissionOutcome {
                role: "backend".to_owned(),
                host_role: "host".to_owned(),
                response: BridgeResponse {
                    v: 1,
                    decision: PermissionDecision::Deny,
                    scope: DecisionScope::Once,
                    reason: "test".to_owned(),
                },
            },
        ]
    }

    fn sample_work_card() -> WorkCard {
        WorkCard {
            role: "backend".to_owned(),
            host_role: "host".to_owned(),
            role_color: Color::Cyan,
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
