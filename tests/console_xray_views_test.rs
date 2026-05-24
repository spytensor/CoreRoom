//! CREP logs and WorkOrder Xray console view fixtures.

use coreroom::console_snapshot::{CoreRoomSnapshot, StatusState};
use coreroom::console_views::{
    build_crep_logs_view, build_workorder_xray_view, CrepLogFilter, WorkOrderXrayView,
};
use coreroom::crep::{CrepEvent, TurnOutcome};
use serde_json::json;

fn snapshot() -> CoreRoomSnapshot {
    toml::from_str(include_str!("fixtures/console_snapshot_v08.toml")).expect("snapshot")
}

#[test]
fn crep_logs_view_renders_event_stream_with_role_thread_and_type_filters() {
    let events = crep_events();
    let rows = build_crep_logs_view(&events, &CrepLogFilter::default());
    assert_eq!(rows.len(), 5);
    assert!(rows.iter().any(|row| row.event_type == "role_started"));
    assert!(rows
        .iter()
        .any(|row| row.event_type == "permission_denied" && row.status == StatusState::Blocking));

    let reviewer_rows = build_crep_logs_view(
        &events,
        &CrepLogFilter {
            role: Some("reviewer".to_owned()),
            ..CrepLogFilter::default()
        },
    );
    assert_eq!(reviewer_rows.len(), 3);
    assert!(reviewer_rows
        .iter()
        .all(|row| row.role.as_deref() == Some("reviewer")));
    assert!(reviewer_rows.iter().all(|row| row.internal));

    let thread_rows = build_crep_logs_view(
        &events,
        &CrepLogFilter {
            thread_id: Some("thread-wo-242".to_owned()),
            ..CrepLogFilter::default()
        },
    );
    assert_eq!(thread_rows.len(), 4);
    assert!(thread_rows
        .iter()
        .all(|row| row.thread_id.as_deref() == Some("thread-wo-242")));

    let denied_rows = build_crep_logs_view(
        &events,
        &CrepLogFilter {
            event_types: vec!["permission_denied".to_owned()],
            ..CrepLogFilter::default()
        },
    );
    assert_eq!(denied_rows.len(), 1);
    assert!(denied_rows[0].summary.contains("denied Bash"));
}

#[test]
fn workorder_xray_preserves_engineering_chain_for_complete_and_stale_paths() {
    let complete = build_workorder_xray_view(&snapshot(), "WO-0241").expect("WO-0241 xray");
    assert_eq!(complete.work_order, "WO-0241");
    assert!(complete.closure_ready);
    assert_eq!(complete.freshness, "complete");
    assert_eq!(
        complete
            .steps
            .iter()
            .map(|step| step.name.as_str())
            .collect::<Vec<_>>(),
        vec![
            "workorder",
            "issue",
            "branch",
            "pr",
            "ci",
            "evidence",
            "tracker",
            "sources"
        ]
    );
    assert_eq!(step(&complete, "pr").value, "#269");
    assert_eq!(step(&complete, "tracker").status, StatusState::Ok);
    assert!(complete.citations.contains(&"pr:#269".to_owned()));

    let stale = build_workorder_xray_view(&snapshot(), "WO-0206").expect("WO-0206 xray");
    assert!(!stale.closure_ready);
    assert_eq!(stale.freshness, "tracker-stale");
    assert_eq!(step(&stale, "tracker").status, StatusState::Blocking);
    assert!(stale.citations.contains(&"tracker:#202".to_owned()));
}

#[test]
fn xray_inspects_internal_delegation_without_polluting_public_transcript() {
    let snapshot = snapshot();
    let xray = build_workorder_xray_view(&snapshot, "WO-0242").expect("WO-0242 xray");
    assert!(xray
        .internal_delegations
        .iter()
        .any(|activity| activity.role == "reviewer"
            && activity
                .xray_ref
                .as_deref()
                .is_some_and(|reference| reference.contains("reviewer"))));
    assert!(xray
        .citations
        .contains(&"source:core-repo@commit:fixture-head".to_owned()));
    assert_eq!(step(&xray, "evidence").freshness, "incomplete");
    assert_eq!(step(&xray, "sources").status, StatusState::Ok);

    let public_text = snapshot
        .conversation
        .public_turns
        .iter()
        .map(|turn| turn.body.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!public_text.contains("Reviewing snapshot schema without entering public transcript"));
}

fn crep_events() -> Vec<CrepEvent> {
    vec![
        CrepEvent::RoleStarted {
            role: "host".to_owned(),
            engine: "cc".to_owned(),
            model: "claude-code".to_owned(),
            session_id: "session-host".to_owned(),
            priors_hash: "sha256:host".to_owned(),
        },
        CrepEvent::TurnDispatched {
            role: "reviewer".to_owned(),
            priors_hash: "sha256:reviewer".to_owned(),
            turn_id: "turn-reviewer-1".to_owned(),
            thread_id: "thread-wo-242".to_owned(),
            parent_turn_id: Some("turn-host-1".to_owned()),
            queue_position: 0,
        },
        CrepEvent::ToolCallProposed {
            role: "reviewer".to_owned(),
            priors_hash: "sha256:reviewer".to_owned(),
            tool_name: "Read".to_owned(),
            tool_input: json!({"file_path":"src/console_views.rs"}),
            tool_use_id: "tool-read-1".to_owned(),
            turn_id: "turn-reviewer-1".to_owned(),
            thread_id: "thread-wo-242".to_owned(),
        },
        CrepEvent::PermissionDenied {
            role: "reviewer".to_owned(),
            priors_hash: "sha256:reviewer".to_owned(),
            tool_name: "Bash".to_owned(),
            tool_input: json!({"command":"git push --force"}),
            reason: "force push is not allowed".to_owned(),
            turn_id: "turn-reviewer-1".to_owned(),
            thread_id: "thread-wo-242".to_owned(),
        },
        CrepEvent::RoleSpoke {
            role: "host".to_owned(),
            priors_hash: "sha256:host".to_owned(),
            text: "Public host summary stays clear.".to_owned(),
            mentions: Vec::new(),
            cost_usd: 0.01,
            cache_read: 128,
            turn_id: "turn-host-2".to_owned(),
            thread_id: "thread-wo-242".to_owned(),
            outcome: TurnOutcome::Converged,
            phase_block: None,
        },
    ]
}

fn step<'a>(xray: &'a WorkOrderXrayView, name: &str) -> &'a coreroom::console_views::XrayStep {
    xray.steps
        .iter()
        .find(|step| step.name == name)
        .expect("xray step")
}
