//! Integration tests for the v0.10 per-spawn lifecycle tracker
//! (issue #380). The unit tests inside `src/spawn_lifecycle.rs` cover
//! the state machine in isolation; this file exercises the public
//! API surface that downstream renderers (#381–#386) and the
//! `RoomRuntimeState` host actually depend on:
//!
//! - Types are re-exported through `console_snapshot` so renderers
//!   can use a single import path.
//! - `RoomRuntimeState::apply_event` feeds the tracker on every
//!   `RoomEvent::Crep`, in lock-step with the existing
//!   rail-rendering pipeline.
//! - The full clean-path lifecycle (`Spawning → Working → Done →
//!   Reported`) and the interrupted-path lifecycle reach the
//!   expected terminal states through realistic event streams.

use std::path::PathBuf;

use coreroom::adapter::Engine;
use coreroom::console_room_runtime::{RoomRuntimeState, TeamMember};
use coreroom::console_snapshot::{
    Outcome, SpawnInstance, SpawnLifecycleTracker, SpawnState, ToolCallStatus,
};
use coreroom::crep::{CrepEvent, InterruptSource, TurnOutcome};
use coreroom::room_io::RoomEvent;
use coreroom::turn::TurnId;

fn make_state() -> RoomRuntimeState {
    // Constructed via the existing `for_project` path against a
    // throwaway tmp dir — the room runtime tolerates a missing
    // `.coreroom/` and falls back to a host-only composer. We just
    // need a real `RoomRuntimeState` to verify the lifecycle wiring.
    RoomRuntimeState::for_project(&PathBuf::from("/tmp/coreroom-380-test"))
}

fn crep_event(event: CrepEvent) -> RoomEvent {
    RoomEvent::Crep {
        event: Box::new(event),
        host_role: "host".to_owned(),
    }
}

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
        source: InterruptSource::UserHalt,
        partial_text: None,
        partial_mentions: Vec::new(),
    }
}

#[test]
fn snapshot_api_re_exports_lifecycle_types() {
    // The issue's AC-1 requires the lifecycle types to be exported
    // through the snapshot API used by the renderer. Importing them
    // via `console_snapshot::` (above) is enough to verify the
    // re-export; this assertion just ensures the types compose into
    // the expected default values.
    let tracker = SpawnLifecycleTracker::new("host");
    assert_eq!(tracker.working_count(), 0);
    assert!(tracker.working_instances_ordered_by_started_at().is_empty());
    assert!(tracker.instances().next().is_none());
    // Outcome must default to Clean so renderers can match
    // unconditionally — only meaningful once Done, per ADR.
    assert_eq!(Outcome::default(), Outcome::Clean);
}

#[test]
fn room_runtime_apply_event_drives_lifecycle_through_clean_path() {
    // Full clean lifecycle through the public `RoomRuntimeState`
    // surface: TurnDispatched → ToolCallProposed → ToolCallExecuted
    // → RoleSpoke (Done) → RoleSpoke (Reported).
    let mut state = make_state();
    state.apply_event(crep_event(turn_dispatched("backend", "t1", None)));
    let tracker = state.spawn_lifecycle();
    let spawn = tracker.instances().next().expect("instance exists");
    assert_eq!(spawn.state, SpawnState::Spawning);
    assert_eq!(spawn.role, "backend");

    state.apply_event(crep_event(tool_proposed("t1", "Bash", "u1")));
    let spawn = state.spawn_lifecycle().instances().next().unwrap();
    assert_eq!(spawn.state, SpawnState::Working);
    assert_eq!(spawn.tool_calls.len(), 1);
    assert_eq!(spawn.tool_calls[0].status, ToolCallStatus::InProgress);

    state.apply_event(crep_event(tool_executed("t1", "u1", true)));
    let spawn = state.spawn_lifecycle().instances().next().unwrap();
    assert_eq!(spawn.tool_calls[0].status, ToolCallStatus::Done);
    assert_eq!(spawn.step_count, 1);

    state.apply_event(crep_event(role_spoke("backend", "t1")));
    let spawn = state.spawn_lifecycle().instances().next().unwrap();
    assert_eq!(spawn.state, SpawnState::Done);
    assert_eq!(spawn.outcome, Outcome::Clean);

    state.apply_event(crep_event(role_spoke("backend", "t1")));
    let spawn = state.spawn_lifecycle().instances().next().unwrap();
    assert_eq!(spawn.state, SpawnState::Reported);
    assert_eq!(spawn.outcome, Outcome::Clean);
}

#[test]
fn room_runtime_apply_event_drives_lifecycle_through_interrupt_path() {
    // Failure path: interrupt mid-Working resolves to Done with
    // Outcome::Interrupted. There is no `Failed` lifecycle state
    // (per ADR §3 in `docs/v0.10-chat-stream-vs-dashboard.md`).
    let mut state = make_state();
    state.apply_event(crep_event(turn_dispatched("security", "t1", None)));
    state.apply_event(crep_event(tool_proposed("t1", "Read", "u1")));
    state.apply_event(crep_event(turn_interrupted("t1")));

    let spawn = state.spawn_lifecycle().instances().next().unwrap();
    assert_eq!(spawn.state, SpawnState::Done);
    assert_eq!(spawn.outcome, Outcome::Interrupted);
    // The synthesized tool record is still InProgress because the
    // interrupt landed before the executed event; that's fine —
    // step_count tracks completed calls only.
    assert_eq!(spawn.step_count, 0);
}

#[test]
fn room_runtime_apply_event_routes_concurrent_spawns_by_turn_id() {
    // AC-3 at the runtime layer: two concurrent dispatches of the
    // same role result in two independent SpawnInstances, each with
    // its own tool-call stream.
    let mut state = make_state();
    state.apply_event(crep_event(turn_dispatched("worker", "t1", None)));
    state.apply_event(crep_event(turn_dispatched("worker", "t2", None)));
    state.apply_event(crep_event(tool_proposed("t1", "Bash", "u1")));
    state.apply_event(crep_event(tool_proposed("t2", "Read", "u2")));

    let instances: Vec<&SpawnInstance> = state.spawn_lifecycle().instances().collect();
    assert_eq!(instances.len(), 2);
    let t1 = instances
        .iter()
        .find(|spawn| spawn.turn_id == "t1")
        .unwrap();
    let t2 = instances
        .iter()
        .find(|spawn| spawn.turn_id == "t2")
        .unwrap();
    assert_ne!(t1.spawn_id, t2.spawn_id);
    assert_eq!(t1.tool_calls[0].tool, "Bash");
    assert_eq!(t2.tool_calls[0].tool, "Read");

    // Both are currently Working.
    assert_eq!(state.spawn_lifecycle().working_count(), 2);
    assert_eq!(state.working_spawn_instances().len(), 2);
}

#[test]
fn room_runtime_working_spawn_instances_excludes_spawning() {
    // ADR footer rule: only `Working` counts toward the N total,
    // `Spawning` does not. The `working_spawn_instances` shortcut
    // on `RoomRuntimeState` must honor that filter.
    let mut state = make_state();
    state.apply_event(crep_event(turn_dispatched("backend", "t1", None)));
    state.apply_event(crep_event(turn_dispatched("security", "t2", None)));
    // Promote only the second to Working with a tool call.
    state.apply_event(crep_event(tool_proposed("t2", "Read", "u1")));

    let working = state.working_spawn_instances();
    assert_eq!(working.len(), 1);
    assert_eq!(working[0].role, "security");
}

#[test]
fn room_runtime_lifecycle_tolerates_unrelated_room_events() {
    // Spinner / WorkCard / Notice events must not crash the
    // tracker. They flow through `apply_event` but only `Crep`
    // events touch the lifecycle.
    use coreroom::room_io::{NoticeLevel, SpinnerPaint, SpinnerSnapshot};
    use std::time::Instant;

    let mut state = make_state();
    state.apply_event(crep_event(turn_dispatched("backend", "t1", None)));
    state.apply_event(RoomEvent::Spinner(SpinnerSnapshot {
        role: "backend".to_owned(),
        frame: 0,
        started_at: Instant::now(),
        tools_seen: 0,
        current_state: None,
        paint: SpinnerPaint::Painting,
    }));
    state.apply_event(RoomEvent::Notice {
        level: NoticeLevel::Hint,
        text: "stub".to_owned(),
    });
    let instances: Vec<&SpawnInstance> = state.spawn_lifecycle().instances().collect();
    assert_eq!(instances.len(), 1);
    assert_eq!(instances[0].state, SpawnState::Spawning);
}

#[test]
fn parent_attribution_threads_through_dispatch_chain() {
    // Verify the spawned_by attribution across a real dispatch
    // chain: @host (root) → @backend (root) → @security (child of
    // backend). Tests that the `parent_turn_id` lookup resolves the
    // parent's role correctly when it itself is a tracked spawn.
    let mut state = make_state();
    state.apply_event(crep_event(turn_dispatched("backend", "t1", None)));
    state.apply_event(crep_event(turn_dispatched("security", "t2", Some("t1"))));

    let instances: Vec<&SpawnInstance> = state.spawn_lifecycle().instances().collect();
    let security = instances
        .iter()
        .find(|spawn| spawn.role == "security")
        .unwrap();
    assert_eq!(security.spawned_by, "backend");
    let backend = instances
        .iter()
        .find(|spawn| spawn.role == "backend")
        .unwrap();
    // Root spawn is attributed to the host role.
    assert_eq!(backend.spawned_by, "host");
}

#[test]
fn team_member_struct_unaffected_by_lifecycle_field() {
    // Smoke test: adding `spawn_lifecycle` to `RoomRuntimeState`
    // must not have shifted the public `TeamMember` shape, which
    // downstream consumers (the chat-stream demo binary in #379)
    // rely on.
    let member = TeamMember {
        role: "frontend".to_owned(),
        engine: Engine::Cc,
    };
    assert_eq!(member.role, "frontend");
}
