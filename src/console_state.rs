//! Pure CREP-to-console reducer.
//!
//! This reducer is deliberately print-free. It turns replayed CREP into state
//! that a future full-screen console can render, while preserving a clean
//! public transcript and internal delegation side-rail activity.

use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::console_snapshot::{
    ConversationSnapshot, ConversationTurn, ConversationVisibility, InternalDelegationActivity,
    InternalDelegationState, RoleLaneState, RoleRuntimeSnapshot, RuntimeSnapshot, SessionFreshness,
};
use crate::conversation_visibility::{decide_visibility, ConversationVisibilityInput};
use crate::crep::CrepEvent;
use crate::gate::{GatePhase, PlanReviewDecision};
use crate::turn::TurnId;

/// Report from replaying JSONL into console state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleReplayReport {
    /// Reduced console state.
    pub state: ConsoleState,
    /// Parsed CREP events.
    pub parsed_events: usize,
    /// Malformed non-empty lines skipped during replay.
    pub skipped_malformed: usize,
}

/// Reducible state for the future console.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleState {
    /// Host role.
    pub host_role: String,
    /// Conversation projection.
    pub conversation: ConversationSnapshot,
    /// Role lane state keyed by role.
    pub roles: BTreeMap<String, RoleConsoleState>,
    /// Tool events and permission status keyed by role.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_activity: Vec<ToolActivity>,
    /// Gate events relevant to console status.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gate_activity: Vec<GateActivity>,
    /// Streaming text folded by turn id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub stream_summaries: BTreeMap<String, StreamSummary>,
    turn_visibility: HashMap<TurnId, ConversationVisibility>,
}

impl ConsoleState {
    /// Create an empty reducer state.
    #[must_use]
    pub fn new(host_role: impl Into<String>) -> Self {
        Self {
            host_role: host_role.into(),
            conversation: ConversationSnapshot {
                public_turns: Vec::new(),
                internal_delegation_count: 0,
                internal_activity: Vec::new(),
            },
            roles: BTreeMap::new(),
            tool_activity: Vec::new(),
            gate_activity: Vec::new(),
            stream_summaries: BTreeMap::new(),
            turn_visibility: HashMap::new(),
        }
    }

    /// Apply one CREP event.
    #[allow(clippy::too_many_lines)]
    pub fn apply_event(&mut self, event: &CrepEvent) -> Result<()> {
        match event {
            CrepEvent::RoleStarted {
                role,
                engine,
                model,
                session_id,
                ..
            } => {
                let lane = self.role_lane(role);
                lane.engine = Some(engine.clone());
                lane.model = Some(model.clone());
                lane.session_id = Some(session_id.clone());
                lane.state = RoleConsoleLifecycle::Idle;
                lane.last_activity = Some("started".to_owned());
            }
            CrepEvent::RoleSessionUpdated {
                role, session_id, ..
            } => {
                let lane = self.role_lane(role);
                lane.session_id = Some(session_id.clone());
                lane.last_activity = Some("session updated".to_owned());
            }
            CrepEvent::TurnDispatched {
                role,
                turn_id,
                parent_turn_id,
                queue_position,
                ..
            } => {
                let is_public = role == &self.host_role || parent_turn_id.is_none();
                let visibility = if is_public {
                    ConversationVisibility::PublicTranscript
                } else {
                    ConversationVisibility::InternalDelegation
                };
                self.turn_visibility.insert(turn_id.clone(), visibility);

                let lane = self.role_lane(role);
                lane.state = if *queue_position == 0 {
                    RoleConsoleLifecycle::Working
                } else {
                    RoleConsoleLifecycle::Queued
                };
                lane.current_turn = Some(turn_id.clone());
                lane.last_activity = Some(if *queue_position == 0 {
                    "dispatched".to_owned()
                } else {
                    format!("queued · {queue_position} ahead")
                });

                if visibility == ConversationVisibility::InternalDelegation {
                    self.record_internal_activity(
                        role,
                        None,
                        InternalDelegationState::Dispatched,
                        "Host-managed role delegation dispatched.",
                        Some(format!("turn:{turn_id}")),
                    );
                }
            }
            CrepEvent::WorkTitle {
                role,
                title,
                turn_id,
                ..
            } => {
                let lane = self.role_lane(role);
                lane.work_title = Some(title.clone());
                lane.current_turn = Some(turn_id.clone());
                lane.last_activity = Some(format!("work: {title}"));
            }
            CrepEvent::RoleOutputDelta {
                role,
                text_delta,
                sequence,
                turn_id,
                ..
            } => {
                let summary = self
                    .stream_summaries
                    .entry(turn_id.clone())
                    .or_insert_with(|| StreamSummary {
                        role: role.clone(),
                        chunks: 0,
                        last_sequence: 0,
                        preview: String::new(),
                    });
                summary.chunks = summary.chunks.saturating_add(1);
                summary.last_sequence = *sequence;
                summary.preview.push_str(text_delta);
                truncate_preview(&mut summary.preview);
                self.role_lane(role).last_activity = Some("streaming output".to_owned());
            }
            CrepEvent::RoleSpoke {
                role,
                text,
                turn_id,
                ..
            } => {
                let visibility = self.turn_visibility.get(turn_id).copied().unwrap_or(
                    if role == &self.host_role {
                        ConversationVisibility::PublicTranscript
                    } else {
                        ConversationVisibility::InternalDelegation
                    },
                );
                match visibility {
                    ConversationVisibility::PublicTranscript | ConversationVisibility::SideRail => {
                        self.conversation.public_turns.push(ConversationTurn {
                            speaker: role.clone(),
                            body: text.clone(),
                            visibility,
                        });
                    }
                    ConversationVisibility::InternalDelegation => {
                        self.conversation.internal_delegation_count = self
                            .conversation
                            .internal_delegation_count
                            .saturating_add(1);
                        self.record_internal_activity(
                            role,
                            None,
                            InternalDelegationState::Completed,
                            "Internal role reply captured for host synthesis.",
                            Some(format!("turn:{turn_id}")),
                        );
                    }
                    ConversationVisibility::DebugLog => {}
                }
                let lane = self.role_lane(role);
                lane.state = RoleConsoleLifecycle::Idle;
                lane.last_activity = Some("spoke".to_owned());
            }
            CrepEvent::ToolCallProposed {
                role,
                tool_name,
                tool_use_id,
                turn_id,
                ..
            } => {
                self.tool_activity.push(ToolActivity {
                    role: role.clone(),
                    tool_name: tool_name.clone(),
                    tool_use_id: tool_use_id.clone(),
                    turn_id: Some(turn_id.clone()),
                    state: ToolActivityState::Proposed,
                    summary: "tool proposed".to_owned(),
                });
                let lane = self.role_lane(role);
                lane.tools_seen = lane.tools_seen.saturating_add(1);
                lane.last_activity = Some(format!("proposed {tool_name}"));
            }
            CrepEvent::ToolCallExecuted {
                role,
                tool_use_id,
                ok,
                output_summary,
                turn_id,
                ..
            } => {
                self.tool_activity.push(ToolActivity {
                    role: role.clone(),
                    tool_name: "tool".to_owned(),
                    tool_use_id: tool_use_id.clone(),
                    turn_id: Some(turn_id.clone()),
                    state: if *ok {
                        ToolActivityState::ExecutedOk
                    } else {
                        ToolActivityState::ExecutedFailed
                    },
                    summary: output_summary.clone(),
                });
                self.role_lane(role).last_activity = Some("tool executed".to_owned());
            }
            CrepEvent::PermissionDenied {
                role,
                tool_name,
                reason,
                turn_id,
                ..
            } => {
                self.tool_activity.push(ToolActivity {
                    role: role.clone(),
                    tool_name: tool_name.clone(),
                    tool_use_id: String::new(),
                    turn_id: Some(turn_id.clone()),
                    state: ToolActivityState::PermissionDenied,
                    summary: reason.clone(),
                });
                let lane = self.role_lane(role);
                lane.state = RoleConsoleLifecycle::WaitingApproval;
                lane.last_activity = Some(format!("permission denied: {tool_name}"));
            }
            CrepEvent::PhaseAdvanced {
                thread,
                from,
                to,
                actor,
                ..
            } => self.gate_activity.push(GateActivity {
                thread: thread.clone(),
                phase: *to,
                state: GateActivityState::Advanced,
                summary: format!("{} -> {} by {actor}", from.label(), to.label()),
                public: false,
            }),
            CrepEvent::PhaseBlocked {
                thread,
                phase,
                role,
                reason,
                ..
            } => {
                self.gate_activity.push(GateActivity {
                    thread: thread.clone(),
                    phase: *phase,
                    state: GateActivityState::Blocked,
                    summary: reason.clone(),
                    public: true,
                });
                let decision = decide_visibility(&ConversationVisibilityInput::RoleToHost {
                    role: role.clone(),
                    surfaced_by_host: Some(
                        crate::conversation_visibility::HostSurfaceReason::CriticalRisk,
                    ),
                })?;
                if decision.public_transcript {
                    self.conversation.public_turns.push(ConversationTurn {
                        speaker: role.clone(),
                        body: format!("Blocked {} phase: {reason}", phase.label()),
                        visibility: decision.visibility,
                    });
                }
            }
            CrepEvent::PlanReviewed {
                role,
                decision,
                plan_sha,
                ..
            } => {
                let public = matches!(
                    decision,
                    PlanReviewDecision::Reject | PlanReviewDecision::NeedsRevision
                );
                self.gate_activity.push(GateActivity {
                    thread: "plan-review".to_owned(),
                    phase: GatePhase::Review,
                    state: match decision {
                        PlanReviewDecision::Approve => GateActivityState::Reviewed,
                        PlanReviewDecision::Reject | PlanReviewDecision::NeedsRevision => {
                            GateActivityState::Blocked
                        }
                    },
                    summary: format!("{} plan {}", role, decision.label()),
                    public,
                });
                if public {
                    self.conversation.public_turns.push(ConversationTurn {
                        speaker: role.clone(),
                        body: format!("Plan review {} ({})", decision.label(), plan_sha),
                        visibility: ConversationVisibility::PublicTranscript,
                    });
                }
            }
            CrepEvent::PlanOverridden { role, reason, .. } => {
                self.conversation.public_turns.push(ConversationTurn {
                    speaker: self.host_role.clone(),
                    body: format!("Plan review override for @{role}: {reason}"),
                    visibility: ConversationVisibility::PublicTranscript,
                });
            }
            CrepEvent::TurnInterrupted {
                role,
                turn_id,
                partial_text,
                ..
            } => {
                let lane = self.role_lane(role);
                lane.state = RoleConsoleLifecycle::Interrupted;
                lane.last_activity = Some("interrupted".to_owned());
                if let Some(partial_text) = partial_text {
                    self.stream_summaries
                        .entry(turn_id.clone())
                        .or_insert_with(|| StreamSummary {
                            role: role.clone(),
                            chunks: 0,
                            last_sequence: 0,
                            preview: partial_text.clone(),
                        });
                }
            }
            CrepEvent::RoleStopped { role, reason, .. } => {
                let lane = self.role_lane(role);
                lane.state = RoleConsoleLifecycle::Stopped;
                lane.last_activity = Some(format!("stopped: {reason:?}"));
            }
        }
        Ok(())
    }

    /// Project role lanes into a runtime snapshot for console rendering.
    #[must_use]
    pub fn runtime_snapshot(
        &self,
        permission_mode: Option<String>,
        session_state: SessionFreshness,
    ) -> RuntimeSnapshot {
        let roles = self
            .roles
            .values()
            .map(|role| RoleRuntimeSnapshot {
                role: role.role.clone(),
                enabled: true,
                engine: role.engine.clone().unwrap_or_else(|| "unknown".to_owned()),
                model: role.model.clone(),
                permission_mode: permission_mode.clone(),
                session_state,
                priors_freshness: None,
                knowledge_freshness: None,
                state: role.state.into(),
                waiting_approval: role.state == RoleConsoleLifecycle::WaitingApproval,
                current_work_order: None,
                current_gate_phase: None,
                last_activity: role.last_activity.clone(),
            })
            .collect::<Vec<_>>();
        RuntimeSnapshot {
            room_id: None,
            host_role: self.host_role.clone(),
            session_state,
            permission_mode,
            active_role: self
                .roles
                .values()
                .find(|role| role.state == RoleConsoleLifecycle::Working)
                .map(|role| role.role.clone()),
            waiting_approval: roles.iter().any(|role| role.waiting_approval),
            roles,
        }
    }

    fn role_lane(&mut self, role: &str) -> &mut RoleConsoleState {
        self.roles
            .entry(role.to_owned())
            .or_insert_with(|| RoleConsoleState {
                role: role.to_owned(),
                engine: None,
                model: None,
                session_id: None,
                state: RoleConsoleLifecycle::Unknown,
                current_turn: None,
                work_title: None,
                tools_seen: 0,
                last_activity: None,
            })
    }

    fn record_internal_activity(
        &mut self,
        role: &str,
        work_order: Option<String>,
        state: InternalDelegationState,
        summary: impl Into<String>,
        xray_ref: Option<String>,
    ) {
        self.conversation
            .internal_activity
            .push(InternalDelegationActivity {
                role: role.to_owned(),
                work_order,
                state,
                summary: summary.into(),
                xray_ref,
            });
    }
}

/// Reduce a slice of CREP events.
pub fn reduce_events(host_role: &str, events: &[CrepEvent]) -> Result<ConsoleState> {
    let mut state = ConsoleState::new(host_role);
    for event in events {
        state.apply_event(event)?;
    }
    Ok(state)
}

/// Reduce a JSONL CREP fixture. Malformed lines are counted, not evidence.
pub fn reduce_jsonl_lines(host_role: &str, input: &str) -> Result<ConsoleReplayReport> {
    let mut events = Vec::new();
    let mut skipped_malformed = 0usize;
    for line in input.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<CrepEvent>(line) {
            Ok(event) => events.push(event),
            Err(_) => skipped_malformed = skipped_malformed.saturating_add(1),
        }
    }
    let parsed_events = events.len();
    let state = reduce_events(host_role, &events)?;
    Ok(ConsoleReplayReport {
        state,
        parsed_events,
        skipped_malformed,
    })
}

/// One role lane after reduction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RoleConsoleState {
    /// Role name.
    pub role: String,
    /// Engine id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    /// Model id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Engine-native session id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Role lifecycle.
    pub state: RoleConsoleLifecycle,
    /// Current turn id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_turn: Option<TurnId>,
    /// Work title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_title: Option<String>,
    /// Tool count seen for this lane.
    pub tools_seen: usize,
    /// Last compact activity string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity: Option<String>,
}

/// Role lifecycle for reducer state.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum RoleConsoleLifecycle {
    /// State not observed.
    Unknown,
    /// Role is idle.
    Idle,
    /// Turn is queued.
    Queued,
    /// Role is working.
    Working,
    /// Role is waiting on permission/approval.
    WaitingApproval,
    /// Turn was interrupted.
    Interrupted,
    /// Role stopped.
    Stopped,
}

impl From<RoleConsoleLifecycle> for RoleLaneState {
    fn from(value: RoleConsoleLifecycle) -> Self {
        match value {
            RoleConsoleLifecycle::Unknown => Self::Enabled,
            RoleConsoleLifecycle::Idle => Self::Idle,
            RoleConsoleLifecycle::Queued | RoleConsoleLifecycle::Working => Self::Working,
            RoleConsoleLifecycle::WaitingApproval => Self::WaitingApproval,
            RoleConsoleLifecycle::Interrupted => Self::Blocked,
            RoleConsoleLifecycle::Stopped => Self::StaleSession,
        }
    }
}

/// Folded streaming summary for a turn.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StreamSummary {
    /// Role that emitted the stream.
    pub role: String,
    /// Number of deltas folded.
    pub chunks: u64,
    /// Last sequence observed.
    pub last_sequence: u64,
    /// Truncated text preview.
    pub preview: String,
}

/// Tool event row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolActivity {
    /// Role that owns the tool call.
    pub role: String,
    /// Tool name.
    pub tool_name: String,
    /// Engine tool-use id.
    pub tool_use_id: String,
    /// Related turn id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    /// Tool activity state.
    pub state: ToolActivityState,
    /// Compact summary.
    pub summary: String,
}

/// Tool activity state.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum ToolActivityState {
    /// Tool was proposed.
    Proposed,
    /// Tool ran successfully.
    ExecutedOk,
    /// Tool ran and failed.
    ExecutedFailed,
    /// Tool was denied.
    PermissionDenied,
}

/// Gate event row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GateActivity {
    /// Gate/thread id.
    pub thread: String,
    /// Gate phase.
    pub phase: GatePhase,
    /// Gate activity state.
    pub state: GateActivityState,
    /// Compact summary.
    pub summary: String,
    /// Whether this gate event affects public conversation.
    pub public: bool,
}

/// Gate activity state.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum GateActivityState {
    /// Gate advanced.
    Advanced,
    /// Gate was blocked.
    Blocked,
    /// Plan review was recorded.
    Reviewed,
}

fn truncate_preview(preview: &mut String) {
    const MAX_CHARS: usize = 120;
    if preview.chars().count() <= MAX_CHARS {
        return;
    }
    *preview = preview.chars().take(MAX_CHARS).collect::<String>();
    preview.push('…');
}
