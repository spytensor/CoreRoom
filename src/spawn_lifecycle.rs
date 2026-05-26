//! Per-spawn-instance lifecycle data model for the v0.10 chat stream.
//!
//! Per the v0.10 ADR (`docs/v0.10-chat-stream-vs-dashboard.md`), every
//! spawned sub-agent flows through four states — `Spawning → Working →
//! Done → Reported` — and each spawn instance must be addressable
//! independently of the role name so concurrent spawns by the same role
//! can be tracked, focused, and interrupted separately. The right-rail
//! `spinners: BTreeMap<String, SpinnerSnapshot>` model on
//! [`crate::console_room_runtime::RoomRuntimeState`] is keyed by role
//! and cannot express any of that.
//!
//! This module is the **data model only** for that lifecycle. It does
//! not render anything; the existing right-rail renderer continues to
//! read from the legacy `spinners` map (preserves the no-visual-diff
//! AC) and the new working-card widget in `#381` consumes this
//! lifecycle data instead.
//!
//! ## Lifecycle vocabulary
//!
//! - [`SpawnState::Spawning`] — sub-agent has been delegated but has
//!   not yet emitted any work signal (no tool call, no streamed text).
//! - [`SpawnState::Working`] — sub-agent has emitted at least one
//!   work signal (tool call or spinner snapshot).
//! - [`SpawnState::Done`] — sub-agent finished its turn (clean,
//!   interrupted, or errored). Per the ADR there is no separate
//!   `Failed` state: every `Done` carries an always-present
//!   [`Outcome`] discriminator with variants `Clean | Interrupted |
//!   Failed`. Renderers single-match on it to draw `✓ done` vs
//!   `⨯ interrupted` vs `⨯ failed`.
//! - [`SpawnState::Reported`] — the sub-agent emitted its report
//!   message after reaching `Done`. The collapsed done line stays in
//!   scrollback as a header for that report.
//!
//! [`SpawnInstance::outcome`] is `Outcome::Clean` by default. Reading
//! it before the spawn reaches `Done` is meaningless per the ADR —
//! consumers must check [`SpawnInstance::state`] first.

use std::collections::BTreeMap;
use std::time::Instant;

use crate::crep::{CrepEvent, InterruptSource};
use crate::turn::TurnId;

/// Stable per-spawn identifier, unique within the lifetime of one
/// [`SpawnLifecycleTracker`]. Implemented as a monotonic `u64` rather
/// than a UUID because (a) the tracker is in-process and never
/// persisted — there is no merge across processes that would need a
/// globally-unique id — and (b) `u64` keys keep [`BTreeMap`] traversal
/// cheap and produce stable ordering for tests and snapshots without
/// pulling in the `uuid` crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SpawnId(u64);

impl SpawnId {
    /// Inner counter value. Exposed for snapshot serialization and
    /// fixture tests; consumers should treat it as opaque.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// The four-state sub-agent lifecycle locked by the v0.10 ADR.
///
/// Failure is **not** a state — it is recorded on
/// [`SpawnInstance::outcome`] (always present once
/// [`Self::Done`]). See the ADR's "Locked vocabulary" and
/// "Resolved open questions" §3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnState {
    /// Sub-agent has been delegated but has not yet emitted work.
    Spawning,
    /// Sub-agent is producing work (tool calls or streamed text).
    Working,
    /// Sub-agent finished its turn (clean, interrupted, or errored).
    Done,
    /// Sub-agent has emitted its report message after `Done`.
    Reported,
}

/// Always-present discriminator on a `Done` spawn that distinguishes a
/// clean completion from an interruption or an error.
///
/// Renderers single-match on this enum once [`SpawnInstance::state`]
/// is [`SpawnState::Done`] or [`SpawnState::Reported`]:
///
/// - [`Self::Clean`] → `@role ✓ done · {elapsed} · {N steps} · [e]xpand log`
/// - [`Self::Interrupted`] → `@role ⨯ interrupted · {elapsed} · {N steps}`
/// - [`Self::Failed`] → `@role ⨯ failed · {elapsed} · {N steps} · [e]xpand log`
///
/// The ADR did not adopt `ToolError(String)` or `Timeout` as outcome
/// variants. Failure context (the underlying tool error message) lives
/// in the last [`ToolCallRecord::summary`] inside
/// [`SpawnInstance::tool_calls`], not on this discriminator.
///
/// Reading [`SpawnInstance::outcome`] before the spawn reaches
/// [`SpawnState::Done`] is meaningless — it carries [`Self::Clean`]
/// by default and only becomes authoritative on the `Working → Done`
/// transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Spawn finished its turn without error or interruption.
    Clean,
    /// Spawn was cancelled by user, peer, or system before completing.
    Interrupted,
    /// Spawn errored out (e.g. terminal tool failure that halted the
    /// turn). The triggering message is recorded in the trailing
    /// [`ToolCallRecord::summary`], not on this variant.
    Failed,
}

impl Default for Outcome {
    /// Default to `Clean` so [`SpawnInstance::outcome`] is always
    /// readable. Per the ADR, consumers must gate on
    /// [`SpawnInstance::state`] reaching [`SpawnState::Done`] before
    /// trusting this field.
    fn default() -> Self {
        Self::Clean
    }
}

/// Status of one tool call within a spawn instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCallStatus {
    /// Tool call has been proposed and is awaiting execution.
    InProgress,
    /// Tool call ran successfully.
    Done,
    /// Tool call ran and failed (or was denied by the permission gate).
    Failed,
}

/// One tool call observed during a spawn instance's `Working` phase.
///
/// Records are append-only — finished tool calls keep their position in
/// [`SpawnInstance::tool_calls`] and gain a `finished_at` timestamp
/// plus a terminal [`ToolCallStatus`]. Renderers iterate to draw a
/// live tool-call stream and derive
/// [`SpawnInstance::step_count`] for the collapsed-done line.
#[derive(Debug, Clone)]
pub struct ToolCallRecord {
    /// Engine-issued tool-use id used to pair the proposal with its
    /// later executed/denied event.
    pub tool_use_id: String,
    /// Engine-native tool identifier (e.g. `Bash`, `Edit`, `Read`).
    pub tool: String,
    /// One-line summary suitable for the live tool-call stream.
    pub summary: String,
    /// Wall-clock instant the tool call was proposed.
    pub started_at: Instant,
    /// Wall-clock instant the tool call finished (executed or denied).
    /// `None` while the call is still [`ToolCallStatus::InProgress`].
    pub finished_at: Option<Instant>,
    /// Terminal status; `InProgress` until the executed/denied event
    /// lands.
    pub status: ToolCallStatus,
}

/// One spawned sub-agent instance — the unit of work tracked by the
/// chat-stream renderer in `#381`.
///
/// Fields are intentionally public so the renderer can read them
/// directly without forcing every accessor through the tracker; the
/// tracker owns mutation, the renderer owns presentation.
#[derive(Debug, Clone)]
pub struct SpawnInstance {
    /// Stable id for the lifetime of this spawn.
    pub spawn_id: SpawnId,
    /// Spawned sub-agent's role name (without the leading `@`).
    pub role: String,
    /// Parent agent's role name. Used by the footer narration line
    /// (`#382`) — never used for visual nesting in the stream.
    pub spawned_by: String,
    /// Current lifecycle state.
    pub state: SpawnState,
    /// Wall-clock instant the spawn was registered (state =
    /// `Spawning`).
    pub started_at: Instant,
    /// Wall-clock instant of the most recent state transition. Used by
    /// renderers to render `elapsed` on the collapsed-done line.
    pub state_changed_at: Instant,
    /// Append-only tool-call records for the spawn's `Working` phase.
    pub tool_calls: Vec<ToolCallRecord>,
    /// Count of completed tool calls. Maintained eagerly so renderers
    /// can read it without scanning `tool_calls` on every frame.
    pub step_count: usize,
    /// Final report message id once the spawn reaches `Reported`.
    /// Populated by the consumer that maps `RoleSpoke` to a renderable
    /// chat message id — left unset by the tracker until it learns the
    /// concrete id type (which is owned by the chat-stream widget).
    pub final_report_message_id: Option<String>,
    /// Always-present completion discriminator. Meaningless until
    /// [`Self::state`] is [`SpawnState::Done`] or
    /// [`SpawnState::Reported`]; defaults to [`Outcome::Clean`]
    /// otherwise. Renderers single-match on this once `Done` to swap
    /// between `✓ done`, `⨯ interrupted`, and `⨯ failed`.
    pub outcome: Outcome,
    /// Turn id that owns this spawn. Used internally to route
    /// follow-up events (tool calls, `RoleSpoke`) to the correct
    /// instance when the same role has multiple concurrent spawns.
    pub turn_id: TurnId,
    /// Conversation-thread id of the parent turn that delegated this
    /// spawn, when one was attributed at dispatch time. Carried so a
    /// future renderer can group spawns under the spawner's chat
    /// thread without re-reading the original `TurnDispatched` event.
    pub parent_turn_id: Option<TurnId>,
}

/// In-memory tracker that owns the [`SpawnInstance`] map and applies
/// kernel `CrepEvent`s to it.
///
/// The tracker is intentionally renderer-agnostic — it knows nothing
/// about ratatui, scrollback, or work cards. The
/// [`crate::console_room_runtime::RoomRuntimeState`] holds one tracker
/// and feeds it every `CrepEvent` it sees; future renderers pull
/// [`SpawnInstance`]s out of [`Self::instances`] and
/// [`Self::working_instances_ordered_by_started_at`].
#[derive(Debug, Default)]
pub struct SpawnLifecycleTracker {
    /// Monotonic counter feeding [`SpawnId`].
    next_id: u64,
    /// All known spawn instances keyed by id. `BTreeMap` (not
    /// `HashMap`) so iteration order is deterministic — important for
    /// tests and for the footer narration which renders working roles
    /// in a stable order when [`Self::working_instances_ordered_by_started_at`]
    /// is not used.
    instances: BTreeMap<SpawnId, SpawnInstance>,
    /// Turn-id → spawn-id index. Lets tool-call and `RoleSpoke`
    /// events find their spawn instance in O(log N) without scanning.
    by_turn: BTreeMap<TurnId, SpawnId>,
    /// Host role name used to attribute root spawns (no `parent_turn_id`).
    /// Root spawns are still recorded so the footer can narrate "host
    /// is working" without a special case in the renderer.
    host_role: String,
}

impl SpawnLifecycleTracker {
    /// Build a new tracker. `host_role` is used as the default
    /// `spawned_by` attribution for root-level spawns.
    #[must_use]
    pub fn new(host_role: impl Into<String>) -> Self {
        Self {
            next_id: 0,
            instances: BTreeMap::new(),
            by_turn: BTreeMap::new(),
            host_role: host_role.into(),
        }
    }

    /// All known spawn instances, in creation order. Renderers may
    /// filter by [`SpawnInstance::state`] as needed.
    pub fn instances(&self) -> impl Iterator<Item = &SpawnInstance> {
        self.instances.values()
    }

    /// Look up one spawn instance by id, if present.
    #[must_use]
    pub fn get(&self, spawn_id: SpawnId) -> Option<&SpawnInstance> {
        self.instances.get(&spawn_id)
    }

    /// Currently-`Working` instances, ordered by
    /// [`SpawnInstance::started_at`]. This is the ordering the footer
    /// narration line (`#382`) needs: only `Working` instances count
    /// toward the `N roles still working` total per the ADR.
    /// `Spawning` instances are intentionally excluded — the consumer
    /// lists them with a `· @role spawning` suffix that does not
    /// increment the count.
    #[must_use]
    pub fn working_instances_ordered_by_started_at(&self) -> Vec<&SpawnInstance> {
        let mut working: Vec<&SpawnInstance> = self
            .instances
            .values()
            .filter(|spawn| spawn.state == SpawnState::Working)
            .collect();
        working.sort_by_key(|spawn| spawn.started_at);
        working
    }

    /// Number of currently-`Working` spawns. Cheaper than
    /// [`Self::working_instances_ordered_by_started_at`] when the
    /// caller only needs the count for a header badge or the footer
    /// `N roles still working` total. Excludes `Spawning` per the
    /// ADR.
    #[must_use]
    pub fn working_count(&self) -> usize {
        self.instances
            .values()
            .filter(|spawn| spawn.state == SpawnState::Working)
            .count()
    }

    /// Currently-`Spawning` instances, ordered by
    /// [`SpawnInstance::started_at`]. Footer narration uses this to
    /// append `· @role spawning` suffixes after the `Working` list
    /// without incrementing the count.
    #[must_use]
    pub fn spawning_instances_ordered_by_started_at(&self) -> Vec<&SpawnInstance> {
        let mut spawning: Vec<&SpawnInstance> = self
            .instances
            .values()
            .filter(|spawn| spawn.state == SpawnState::Spawning)
            .collect();
        spawning.sort_by_key(|spawn| spawn.started_at);
        spawning
    }

    /// Apply one kernel event to the tracker. Unknown / irrelevant
    /// events are ignored. Returns the affected [`SpawnId`] when the
    /// event mutated a spawn instance, for callers that want to log
    /// or test specific transitions.
    pub fn apply_event(&mut self, event: &CrepEvent) -> Option<SpawnId> {
        match event {
            CrepEvent::TurnDispatched {
                role,
                turn_id,
                parent_turn_id,
                ..
            } => Some(self.on_turn_dispatched(role, turn_id, parent_turn_id.as_ref())),
            CrepEvent::ToolCallProposed {
                tool_name,
                tool_use_id,
                turn_id,
                ..
            } => self.on_tool_proposed(turn_id, tool_name, tool_use_id),
            CrepEvent::ToolCallExecuted {
                tool_use_id,
                ok,
                output_summary,
                turn_id,
                ..
            } => self.on_tool_executed(turn_id, tool_use_id, *ok, output_summary),
            CrepEvent::PermissionDenied {
                tool_name,
                reason,
                turn_id,
                ..
            } => self.on_permission_denied(turn_id, tool_name, reason),
            CrepEvent::RoleOutputDelta { turn_id, .. } => self.on_output_delta(turn_id),
            CrepEvent::RoleSpoke { turn_id, .. } => self.on_role_spoke(turn_id),
            CrepEvent::TurnInterrupted {
                turn_id, source, ..
            } => self.on_turn_interrupted(turn_id, *source),
            // Events that do not affect a per-spawn lifecycle record.
            CrepEvent::RoleStarted { .. }
            | CrepEvent::RoleSessionUpdated { .. }
            | CrepEvent::WorkTitle { .. }
            | CrepEvent::PhaseAdvanced { .. }
            | CrepEvent::PhaseBlocked { .. }
            | CrepEvent::PlanReviewed { .. }
            | CrepEvent::PlanOverridden { .. }
            | CrepEvent::RoleStopped { .. } => None,
        }
    }

    fn next_spawn_id(&mut self) -> SpawnId {
        let id = SpawnId(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    fn on_turn_dispatched(
        &mut self,
        role: &str,
        turn_id: &TurnId,
        parent_turn_id: Option<&TurnId>,
    ) -> SpawnId {
        // Concurrent spawns by the same role: each `TurnDispatched`
        // mints a new spawn id even if a prior spawn for the same role
        // is still `Working`. Distinguishing comes from `turn_id`.
        let spawn_id = self.next_spawn_id();
        let spawned_by = parent_turn_id
            .and_then(|parent| self.by_turn.get(parent).copied())
            .and_then(|parent_id| self.instances.get(&parent_id))
            .map_or_else(|| self.host_role.clone(), |parent| parent.role.clone());
        let now = Instant::now();
        let instance = SpawnInstance {
            spawn_id,
            role: role.to_owned(),
            spawned_by,
            state: SpawnState::Spawning,
            started_at: now,
            state_changed_at: now,
            tool_calls: Vec::new(),
            step_count: 0,
            final_report_message_id: None,
            outcome: Outcome::default(),
            turn_id: turn_id.clone(),
            parent_turn_id: parent_turn_id.cloned(),
        };
        self.instances.insert(spawn_id, instance);
        self.by_turn.insert(turn_id.clone(), spawn_id);
        spawn_id
    }

    fn on_tool_proposed(
        &mut self,
        turn_id: &TurnId,
        tool_name: &str,
        tool_use_id: &str,
    ) -> Option<SpawnId> {
        let spawn_id = self.by_turn.get(turn_id).copied()?;
        let instance = self.instances.get_mut(&spawn_id)?;
        let now = Instant::now();
        instance.tool_calls.push(ToolCallRecord {
            tool_use_id: tool_use_id.to_owned(),
            tool: tool_name.to_owned(),
            summary: String::new(),
            started_at: now,
            finished_at: None,
            status: ToolCallStatus::InProgress,
        });
        // First work signal promotes Spawning → Working.
        if instance.state == SpawnState::Spawning {
            instance.state = SpawnState::Working;
            instance.state_changed_at = now;
        }
        Some(spawn_id)
    }

    fn on_tool_executed(
        &mut self,
        turn_id: &TurnId,
        tool_use_id: &str,
        ok: bool,
        summary: &str,
    ) -> Option<SpawnId> {
        let spawn_id = self.by_turn.get(turn_id).copied()?;
        let instance = self.instances.get_mut(&spawn_id)?;
        let now = Instant::now();
        if let Some(record) = instance
            .tool_calls
            .iter_mut()
            .rev()
            .find(|record| record.tool_use_id == tool_use_id)
        {
            record.finished_at = Some(now);
            summary.clone_into(&mut record.summary);
            record.status = if ok {
                ToolCallStatus::Done
            } else {
                ToolCallStatus::Failed
            };
        }
        instance.step_count = instance
            .tool_calls
            .iter()
            .filter(|record| !matches!(record.status, ToolCallStatus::InProgress))
            .count();
        // A finished tool call also counts as a work signal — covers
        // the edge case where a proposal was filtered out upstream
        // (e.g. by a denylist test fixture) but the executed event
        // still arrived.
        if instance.state == SpawnState::Spawning {
            instance.state = SpawnState::Working;
            instance.state_changed_at = now;
        }
        Some(spawn_id)
    }

    fn on_permission_denied(
        &mut self,
        turn_id: &TurnId,
        tool_name: &str,
        reason: &str,
    ) -> Option<SpawnId> {
        let spawn_id = self.by_turn.get(turn_id).copied()?;
        let instance = self.instances.get_mut(&spawn_id)?;
        let now = Instant::now();
        // A denied proposal may not have been recorded as a separate
        // `ToolCallProposed` event in some adapter paths; synthesize
        // a one-shot record so the renderer sees the denial.
        instance.tool_calls.push(ToolCallRecord {
            tool_use_id: String::new(),
            tool: tool_name.to_owned(),
            summary: reason.to_owned(),
            started_at: now,
            finished_at: Some(now),
            status: ToolCallStatus::Failed,
        });
        instance.step_count = instance
            .tool_calls
            .iter()
            .filter(|record| !matches!(record.status, ToolCallStatus::InProgress))
            .count();
        if instance.state == SpawnState::Spawning {
            instance.state = SpawnState::Working;
            instance.state_changed_at = now;
        }
        Some(spawn_id)
    }

    fn on_output_delta(&mut self, turn_id: &TurnId) -> Option<SpawnId> {
        let spawn_id = self.by_turn.get(turn_id).copied()?;
        let instance = self.instances.get_mut(&spawn_id)?;
        if instance.state == SpawnState::Spawning {
            // Streaming text without a tool call is also a "I'm
            // working" signal.
            instance.state = SpawnState::Working;
            instance.state_changed_at = Instant::now();
        }
        Some(spawn_id)
    }

    fn on_role_spoke(&mut self, turn_id: &TurnId) -> Option<SpawnId> {
        let spawn_id = self.by_turn.get(turn_id).copied()?;
        let instance = self.instances.get_mut(&spawn_id)?;
        let now = Instant::now();
        match instance.state {
            // First `RoleSpoke` for the turn → `Done`. Even if no
            // tool call was observed (a one-shot reply), we still go
            // through `Working` semantically; renderers branch on the
            // step count, not on whether they saw `Working` first.
            // `outcome` stays at its default `Clean` for a successful
            // `RoleSpoke` — the only callers that promote it to
            // `Interrupted` / `Failed` are `on_turn_interrupted` and
            // future error paths.
            SpawnState::Spawning | SpawnState::Working => {
                instance.state = SpawnState::Done;
                instance.state_changed_at = now;
            }
            // Second `RoleSpoke` after `Done` → `Reported`.
            SpawnState::Done => {
                instance.state = SpawnState::Reported;
                instance.state_changed_at = now;
            }
            SpawnState::Reported => {}
        }
        Some(spawn_id)
    }

    fn on_turn_interrupted(
        &mut self,
        turn_id: &TurnId,
        _source: InterruptSource,
    ) -> Option<SpawnId> {
        let spawn_id = self.by_turn.get(turn_id).copied()?;
        let instance = self.instances.get_mut(&spawn_id)?;
        let now = Instant::now();
        if matches!(instance.state, SpawnState::Spawning | SpawnState::Working) {
            instance.state = SpawnState::Done;
            instance.state_changed_at = now;
            // ADR: outcome is the renderer's single match — interrupt
            // always maps to `Interrupted`, regardless of which actor
            // requested it. Per-source labels can be re-derived by the
            // renderer from the underlying `CrepEvent::TurnInterrupted`
            // if a tailored hint is ever needed.
            instance.outcome = Outcome::Interrupted;
        }
        Some(spawn_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crep::{InterruptSource, StopReason, TurnOutcome};
    use crate::turn::TurnId;

    fn turn_dispatched(role: &str, turn: &str, parent: Option<&str>) -> CrepEvent {
        CrepEvent::TurnDispatched {
            role: role.to_owned(),
            priors_hash: String::new(),
            turn_id: TurnId::from(turn.to_owned()),
            thread_id: TurnId::from(format!("thread-{turn}")),
            parent_turn_id: parent.map(|p| TurnId::from(p.to_owned())),
            queue_position: 0,
        }
    }

    fn tool_proposed(turn: &str, tool: &str, tool_use_id: &str) -> CrepEvent {
        CrepEvent::ToolCallProposed {
            role: String::new(),
            priors_hash: String::new(),
            tool_name: tool.to_owned(),
            tool_input: serde_json::json!({}),
            tool_use_id: tool_use_id.to_owned(),
            turn_id: TurnId::from(turn.to_owned()),
            thread_id: TurnId::from(format!("thread-{turn}")),
        }
    }

    fn tool_executed(turn: &str, tool_use_id: &str, ok: bool) -> CrepEvent {
        CrepEvent::ToolCallExecuted {
            role: String::new(),
            priors_hash: String::new(),
            tool_use_id: tool_use_id.to_owned(),
            ok,
            output_summary: if ok {
                "ok".to_owned()
            } else {
                "err".to_owned()
            },
            turn_id: TurnId::from(turn.to_owned()),
            thread_id: TurnId::from(format!("thread-{turn}")),
        }
    }

    fn role_spoke(role: &str, turn: &str) -> CrepEvent {
        CrepEvent::RoleSpoke {
            role: role.to_owned(),
            priors_hash: String::new(),
            text: format!("{role} reply"),
            mentions: Vec::new(),
            cost_usd: 0.0,
            cache_read: 0,
            turn_id: TurnId::from(turn.to_owned()),
            thread_id: TurnId::from(format!("thread-{turn}")),
            outcome: TurnOutcome::default(),
            phase_block: None,
        }
    }

    fn turn_interrupted(turn: &str) -> CrepEvent {
        CrepEvent::TurnInterrupted {
            role: String::new(),
            priors_hash: String::new(),
            turn_id: TurnId::from(turn.to_owned()),
            thread_id: TurnId::from(format!("thread-{turn}")),
            source: InterruptSource::UserCtrlC,
            partial_text: None,
            partial_mentions: Vec::new(),
        }
    }

    #[test]
    fn dispatch_creates_spawning_instance_attributed_to_host_by_default() {
        let mut tracker = SpawnLifecycleTracker::new("host");
        let id = tracker
            .apply_event(&turn_dispatched("backend", "t1", None))
            .expect("dispatch yields spawn id");
        let instance = tracker.get(id).expect("instance exists");
        assert_eq!(instance.role, "backend");
        assert_eq!(instance.spawned_by, "host");
        assert_eq!(instance.state, SpawnState::Spawning);
        // Outcome defaults to `Clean` and is meaningless until Done.
        assert_eq!(instance.outcome, Outcome::Clean);
        assert!(instance.tool_calls.is_empty());
    }

    #[test]
    fn first_tool_call_transitions_spawning_to_working() {
        let mut tracker = SpawnLifecycleTracker::new("host");
        let id = tracker
            .apply_event(&turn_dispatched("backend", "t1", None))
            .unwrap();
        tracker.apply_event(&tool_proposed("t1", "Bash", "u1"));
        let instance = tracker.get(id).unwrap();
        assert_eq!(instance.state, SpawnState::Working);
        assert_eq!(instance.tool_calls.len(), 1);
        assert_eq!(instance.tool_calls[0].status, ToolCallStatus::InProgress);
        assert_eq!(instance.step_count, 0);
    }

    #[test]
    fn streaming_output_alone_also_promotes_to_working() {
        let mut tracker = SpawnLifecycleTracker::new("host");
        let id = tracker
            .apply_event(&turn_dispatched("backend", "t1", None))
            .unwrap();
        tracker.apply_event(&CrepEvent::RoleOutputDelta {
            role: "backend".to_owned(),
            priors_hash: String::new(),
            text_delta: "hi".to_owned(),
            sequence: 0,
            turn_id: TurnId::from("t1".to_owned()),
            thread_id: TurnId::from("thread-t1".to_owned()),
        });
        assert_eq!(tracker.get(id).unwrap().state, SpawnState::Working);
    }

    #[test]
    fn tool_executed_marks_record_done_and_increments_step_count() {
        let mut tracker = SpawnLifecycleTracker::new("host");
        let id = tracker
            .apply_event(&turn_dispatched("backend", "t1", None))
            .unwrap();
        tracker.apply_event(&tool_proposed("t1", "Bash", "u1"));
        tracker.apply_event(&tool_executed("t1", "u1", true));
        let instance = tracker.get(id).unwrap();
        assert_eq!(instance.tool_calls[0].status, ToolCallStatus::Done);
        assert!(instance.tool_calls[0].finished_at.is_some());
        assert_eq!(instance.tool_calls[0].summary, "ok");
        assert_eq!(instance.step_count, 1);
        assert_eq!(instance.state, SpawnState::Working);
    }

    #[test]
    fn failed_tool_call_does_not_introduce_failed_state() {
        // Per the ADR: a failing tool call does NOT push the spawn
        // into a `Failed` state. Lifecycle stays Working until
        // RoleSpoke / TurnInterrupted reaches Done.
        let mut tracker = SpawnLifecycleTracker::new("host");
        let id = tracker
            .apply_event(&turn_dispatched("backend", "t1", None))
            .unwrap();
        tracker.apply_event(&tool_proposed("t1", "Bash", "u1"));
        tracker.apply_event(&tool_executed("t1", "u1", false));
        let instance = tracker.get(id).unwrap();
        assert_eq!(instance.tool_calls[0].status, ToolCallStatus::Failed);
        assert_eq!(instance.step_count, 1);
        assert_eq!(instance.state, SpawnState::Working);
        // Outcome stays at default Clean until the spawn actually
        // reaches Done — the tool failure on its own does not flip it.
        assert_eq!(instance.outcome, Outcome::Clean);
    }

    #[test]
    fn role_spoke_transitions_working_to_done_then_reported_clean() {
        let mut tracker = SpawnLifecycleTracker::new("host");
        let id = tracker
            .apply_event(&turn_dispatched("backend", "t1", None))
            .unwrap();
        tracker.apply_event(&tool_proposed("t1", "Bash", "u1"));
        tracker.apply_event(&tool_executed("t1", "u1", true));
        tracker.apply_event(&role_spoke("backend", "t1"));
        let instance = tracker.get(id).unwrap();
        assert_eq!(instance.state, SpawnState::Done);
        assert_eq!(instance.outcome, Outcome::Clean);
        // A subsequent emission on the same turn moves Done → Reported.
        tracker.apply_event(&role_spoke("backend", "t1"));
        let instance = tracker.get(id).unwrap();
        assert_eq!(instance.state, SpawnState::Reported);
        // Reported preserves the outcome — renderers continue to draw
        // the same `✓` / `⨯` annotation in the collapsed line above
        // the report message.
        assert_eq!(instance.outcome, Outcome::Clean);
        // Once Reported, further events are no-ops on the lifecycle.
        tracker.apply_event(&role_spoke("backend", "t1"));
        assert_eq!(tracker.get(id).unwrap().state, SpawnState::Reported);
    }

    #[test]
    fn turn_interrupted_resolves_to_done_with_interrupted_outcome() {
        let mut tracker = SpawnLifecycleTracker::new("host");
        let id = tracker
            .apply_event(&turn_dispatched("backend", "t1", None))
            .unwrap();
        tracker.apply_event(&tool_proposed("t1", "Bash", "u1"));
        tracker.apply_event(&turn_interrupted("t1"));
        let instance = tracker.get(id).unwrap();
        assert_eq!(instance.state, SpawnState::Done);
        assert_eq!(instance.outcome, Outcome::Interrupted);
    }

    #[test]
    fn interrupt_during_spawning_still_reaches_done_with_interrupted_outcome() {
        let mut tracker = SpawnLifecycleTracker::new("host");
        let id = tracker
            .apply_event(&turn_dispatched("backend", "t1", None))
            .unwrap();
        tracker.apply_event(&turn_interrupted("t1"));
        let instance = tracker.get(id).unwrap();
        assert_eq!(instance.state, SpawnState::Done);
        assert_eq!(instance.outcome, Outcome::Interrupted);
    }

    #[test]
    fn concurrent_spawns_by_same_role_have_distinct_spawn_ids() {
        // AC-3: two `TurnDispatched` events for the same role on
        // different turn ids must mint two distinct spawn instances.
        let mut tracker = SpawnLifecycleTracker::new("host");
        let id_a = tracker
            .apply_event(&turn_dispatched("backend", "t1", None))
            .unwrap();
        let id_b = tracker
            .apply_event(&turn_dispatched("backend", "t2", None))
            .unwrap();
        assert_ne!(id_a, id_b);

        // Tool calls on each turn route to the correct spawn.
        tracker.apply_event(&tool_proposed("t1", "Bash", "u1"));
        tracker.apply_event(&tool_proposed("t2", "Read", "u2"));
        let a = tracker.get(id_a).unwrap();
        let b = tracker.get(id_b).unwrap();
        assert_eq!(a.tool_calls.len(), 1);
        assert_eq!(b.tool_calls.len(), 1);
        assert_eq!(a.tool_calls[0].tool, "Bash");
        assert_eq!(b.tool_calls[0].tool, "Read");

        // Finishing one does not finish the other.
        tracker.apply_event(&role_spoke("backend", "t1"));
        assert_eq!(tracker.get(id_a).unwrap().state, SpawnState::Done);
        assert_eq!(tracker.get(id_b).unwrap().state, SpawnState::Working);
    }

    #[test]
    fn parent_turn_id_resolves_spawned_by_to_parent_role() {
        // @host turn dispatches @backend, which then auto-routes to
        // @security via `parent_turn_id`. The security spawn should be
        // attributed to backend, not host.
        let mut tracker = SpawnLifecycleTracker::new("host");
        let _backend = tracker
            .apply_event(&turn_dispatched("backend", "t1", None))
            .unwrap();
        let security = tracker
            .apply_event(&turn_dispatched("security", "t2", Some("t1")))
            .unwrap();
        assert_eq!(tracker.get(security).unwrap().spawned_by, "backend");
    }

    #[test]
    fn permission_denied_records_failed_tool_without_failing_lifecycle() {
        let mut tracker = SpawnLifecycleTracker::new("host");
        let id = tracker
            .apply_event(&turn_dispatched("backend", "t1", None))
            .unwrap();
        tracker.apply_event(&CrepEvent::PermissionDenied {
            role: "backend".to_owned(),
            priors_hash: String::new(),
            tool_name: "Bash".to_owned(),
            tool_input: serde_json::json!({}),
            reason: "blocked".to_owned(),
            turn_id: TurnId::from("t1".to_owned()),
            thread_id: TurnId::from("thread-t1".to_owned()),
        });
        let instance = tracker.get(id).unwrap();
        assert_eq!(instance.state, SpawnState::Working);
        assert_eq!(instance.tool_calls.len(), 1);
        assert_eq!(instance.tool_calls[0].status, ToolCallStatus::Failed);
        assert_eq!(instance.step_count, 1);
        // Permission denial alone does not flip the outcome —
        // lifecycle only reaches Done with Interrupted / Failed via
        // the turn-end events.
        assert_eq!(instance.outcome, Outcome::Clean);
    }

    #[test]
    fn working_instances_ordered_by_started_at_lists_older_first() {
        let mut tracker = SpawnLifecycleTracker::new("host");
        let first = tracker
            .apply_event(&turn_dispatched("security", "t1", None))
            .unwrap();
        // Force a different started_at by burning a tick. `Instant`
        // is monotonic on every supported platform.
        std::thread::sleep(std::time::Duration::from_millis(2));
        let second = tracker
            .apply_event(&turn_dispatched("backend", "t2", None))
            .unwrap();
        tracker.apply_event(&tool_proposed("t1", "Bash", "u1"));
        tracker.apply_event(&tool_proposed("t2", "Read", "u2"));
        let ordered: Vec<SpawnId> = tracker
            .working_instances_ordered_by_started_at()
            .into_iter()
            .map(|spawn| spawn.spawn_id)
            .collect();
        assert_eq!(ordered, vec![first, second]);
        assert_eq!(tracker.working_count(), 2);
    }

    #[test]
    fn spawning_instances_helper_excludes_working_per_adr_footer_rule() {
        // ADR locks the footer rule: only `Working` counts toward the
        // `N roles still working` total; `Spawning` roles are named
        // with a suffix but not counted. The two helpers must be
        // disjoint by state.
        let mut tracker = SpawnLifecycleTracker::new("host");
        let _spawning = tracker
            .apply_event(&turn_dispatched("backend", "t1", None))
            .unwrap();
        let working = tracker
            .apply_event(&turn_dispatched("security", "t2", None))
            .unwrap();
        // Promote the second spawn to Working with a tool call; leave
        // the first in Spawning.
        tracker.apply_event(&tool_proposed("t2", "Read", "u1"));
        let working_ids: Vec<SpawnId> = tracker
            .working_instances_ordered_by_started_at()
            .into_iter()
            .map(|spawn| spawn.spawn_id)
            .collect();
        let spawning_ids: Vec<SpawnId> = tracker
            .spawning_instances_ordered_by_started_at()
            .into_iter()
            .map(|spawn| spawn.spawn_id)
            .collect();
        assert_eq!(working_ids, vec![working]);
        assert_eq!(spawning_ids.len(), 1);
        assert!(!spawning_ids.contains(&working));
        // Working count must not include the Spawning instance.
        assert_eq!(tracker.working_count(), 1);
    }

    #[test]
    fn unknown_events_are_ignored_and_do_not_mutate_state() {
        // Events that have no per-spawn meaning return `None` and
        // leave the tracker untouched.
        let mut tracker = SpawnLifecycleTracker::new("host");
        tracker.apply_event(&turn_dispatched("backend", "t1", None));
        let before = tracker.get(SpawnId(0)).cloned().unwrap();
        let result = tracker.apply_event(&CrepEvent::RoleStarted {
            role: "backend".to_owned(),
            engine: "cc".to_owned(),
            model: "claude".to_owned(),
            session_id: "s1".to_owned(),
            priors_hash: String::new(),
        });
        assert!(result.is_none());
        let after = tracker.get(SpawnId(0)).unwrap();
        assert_eq!(before.state, after.state);
        assert_eq!(before.tool_calls.len(), after.tool_calls.len());
        // Silences the unused-import warning when the StopReason
        // helper is not pulled into a transition test.
        let _ = StopReason::Completed;
    }
}
