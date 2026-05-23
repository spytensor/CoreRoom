//! Console overview projection fixtures.

use coreroom::console_overview::{build_console_overview, ConsoleOverview, OverviewPulse};
use coreroom::console_snapshot::{
    CoreRoomSnapshot, DirtyState, EvidenceClosureState, HealthSeverity, RoleLaneState,
    SourceHealthState, StatusState, WorkLifecycle,
};

fn snapshot() -> CoreRoomSnapshot {
    toml::from_str(include_str!("fixtures/console_snapshot_v08.toml")).expect("snapshot")
}

#[test]
fn overview_preserves_project_host_and_tracker_facts() {
    let overview = build_console_overview(&snapshot());

    assert_eq!(overview.header.project, "CoreRoom");
    assert_eq!(overview.header.repository, "spytensor/CoreRoom");
    assert_eq!(
        overview.header.branch,
        "feat/v0.8-242-coreroom-snapshot-schema"
    );
    assert_eq!(overview.header.host_role, "host");
    assert_eq!(overview.header.tracker_issue, 238);
    assert_eq!(overview.header.dirty_state, DirtyState::Dirty);
}

#[test]
fn overview_blocked_snapshot_has_actionable_pulses_and_alerts() {
    let overview = build_console_overview(&snapshot());

    assert_pulse(&overview, "Roles", 7, 3, 3, 1, 0);
    assert_pulse(&overview, "WorkOrders", 5, 2, 1, 2, 0);
    assert_pulse(&overview, "Gates", 2, 0, 1, 1, 0);
    assert_pulse(&overview, "Evidence", 2, 1, 1, 0, 0);
    assert_pulse(&overview, "Sources", 2, 1, 1, 0, 0);

    assert!(overview
        .alerts
        .iter()
        .any(|alert| alert.severity == HealthSeverity::Blocking));
    assert!(overview.alerts.iter().all(|alert| {
        alert.severity != HealthSeverity::Ok
            || alert
                .next_action
                .as_deref()
                .is_some_and(|action| !action.is_empty())
    }));
    assert!(overview
        .alerts
        .iter()
        .any(|alert| alert.source == "work:WO-0206"));
}

#[test]
fn overview_healthy_snapshot_has_no_fake_alerts() {
    let mut snapshot = snapshot();
    snapshot.project.dirty_state = DirtyState::Clean;
    snapshot.github.open_issues = 0;
    snapshot.github.open_pull_requests = 0;
    snapshot.github.failing_checks = 0;
    snapshot.alerts.clear();

    for role in &mut snapshot.runtime.roles {
        role.state = RoleLaneState::Idle;
        role.waiting_approval = false;
        role.permission_mode = Some("ask".to_owned());
    }
    for work in &mut snapshot.work {
        work.lifecycle = WorkLifecycle::Closed;
        work.ci_state = StatusState::Ok;
        work.evidence_state = StatusState::Ok;
        work.tracker_state = StatusState::Ok;
    }
    for gate in &mut snapshot.gates {
        gate.blocked_reason = None;
        gate.missing_reviews.clear();
        gate.stale_plan_sha = None;
        gate.signoff_ready = true;
    }
    for evidence in &mut snapshot.evidence {
        evidence.status = EvidenceClosureState::Complete;
        evidence.missing_fields.clear();
        evidence.unverified_items.clear();
        evidence.tracker_updated = true;
    }
    for source in &mut snapshot.sources {
        source.status = SourceHealthState::Pinned;
    }

    let overview = build_console_overview(&snapshot);
    assert!(overview.alerts.is_empty());
    for pulse in overview.pulses {
        assert_eq!(pulse.blocking, 0, "{} blocking", pulse.label);
        assert_eq!(pulse.warn, 0, "{} warn", pulse.label);
        assert_eq!(pulse.unknown, 0, "{} unknown", pulse.label);
        assert!(pulse.next_action.is_none(), "{} next action", pulse.label);
    }
}

fn assert_pulse(
    overview: &ConsoleOverview,
    label: &str,
    total: usize,
    ok: usize,
    warn: usize,
    blocking: usize,
    unknown: usize,
) {
    let pulse = pulse(overview, label);
    assert_eq!(pulse.total, total, "{label} total");
    assert_eq!(pulse.ok, ok, "{label} ok");
    assert_eq!(pulse.warn, warn, "{label} warn");
    assert_eq!(pulse.blocking, blocking, "{label} blocking");
    assert_eq!(pulse.unknown, unknown, "{label} unknown");
    if blocking + warn + unknown > 0 {
        assert!(pulse.next_action.is_some(), "{label} next action");
    }
}

fn pulse<'a>(overview: &'a ConsoleOverview, label: &str) -> &'a OverviewPulse {
    overview
        .pulses
        .iter()
        .find(|pulse| pulse.label == label)
        .expect("pulse")
}
