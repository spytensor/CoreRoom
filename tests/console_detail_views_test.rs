//! Detailed gate, evidence, and source console view fixtures.

use coreroom::console_snapshot::{
    CoreRoomSnapshot, EvidenceClosureState, SourceHealthSnapshot, SourceHealthState, StatusState,
};
use coreroom::console_views::{
    build_evidence_view, build_gates_view, build_sources_view, EvidenceClosureView,
    GateProgressView, SourceHealthView,
};

fn snapshot() -> CoreRoomSnapshot {
    toml::from_str(include_str!("fixtures/console_snapshot_v08.toml")).expect("snapshot")
}

#[test]
fn gates_view_exposes_phase_blockers_reviews_plan_freshness_and_signoff() {
    let gates = build_gates_view(&snapshot());
    let blocked = gate(&gates, "WO-0251");

    assert_eq!(blocked.current_phase, "discovery");
    assert_eq!(blocked.status, StatusState::Blocking);
    assert!(blocked
        .blocked_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("Requires snapshot")));
    assert_eq!(blocked.missing_reviews, vec!["reviewer", "qa"]);
    assert!(!blocked.signoff_ready);
    assert_eq!(blocked.detail.freshness, "fresh");
    assert!(blocked
        .detail
        .next_action
        .as_deref()
        .is_some_and(|action| action.contains("resolve blocked gate")));
    assert!(blocked.citations.contains(&"tracker:#238".to_owned()));

    let mut stale_snapshot = snapshot();
    let stale_gate = stale_snapshot
        .gates
        .iter_mut()
        .find(|gate| gate.work_order == "WO-0242")
        .expect("WO-0242 gate");
    stale_gate.signoff_ready = true;
    stale_gate.stale_plan_sha = Some("sha256:old-plan".to_owned());
    let stale = build_gates_view(&stale_snapshot);
    let stale_row = gate(&stale, "WO-0242");
    assert_eq!(stale_row.status, StatusState::Warn);
    assert_eq!(stale_row.detail.freshness, "stale-plan");
    assert!(stale_row
        .detail
        .next_action
        .as_deref()
        .is_some_and(|action| action.contains("refresh stale plan")));

    let mut healthy_snapshot = snapshot();
    let healthy_gate = healthy_snapshot
        .gates
        .iter_mut()
        .find(|gate| gate.work_order == "WO-0242")
        .expect("WO-0242 gate");
    healthy_gate.signoff_ready = true;
    let healthy = build_gates_view(&healthy_snapshot);
    let healthy_row = gate(&healthy, "WO-0242");
    assert_eq!(healthy_row.status, StatusState::Ok);
    assert!(healthy_row.detail.next_action.is_none());
}

#[test]
fn evidence_view_exposes_missing_fields_unverified_items_rollback_and_tracker_closure() {
    let evidence = build_evidence_view(&snapshot());
    let incomplete = evidence_row(&evidence, "WO-0242");

    assert_eq!(incomplete.status, EvidenceClosureState::Incomplete);
    assert_eq!(
        incomplete.missing_fields,
        vec!["PR", "CI", "tracker update"]
    );
    assert_eq!(
        incomplete.unverified_items,
        vec!["GitHub CI has not run for the snapshot branch yet."]
    );
    assert!(!incomplete.tracker_updated);
    assert_eq!(incomplete.health, StatusState::Blocking);
    assert_eq!(incomplete.detail.freshness, "incomplete");
    assert!(incomplete
        .detail
        .next_action
        .as_deref()
        .is_some_and(|action| action.contains("fill evidence")));
    assert!(incomplete
        .rollback
        .as_deref()
        .is_some_and(|rollback| rollback.contains("console_snapshot")));
    assert!(incomplete.citations.contains(&"issue:#242".to_owned()));

    let complete = evidence_row(&evidence, "WO-0241");
    assert_eq!(complete.status, EvidenceClosureState::Complete);
    assert!(complete.tracker_updated);
    assert_eq!(complete.health, StatusState::Ok);
    assert_eq!(complete.detail.freshness, "complete");
    assert!(complete.detail.closure_ready);
    assert!(complete.detail.next_action.is_none());
}

#[test]
fn sources_view_exposes_pins_trust_visibility_related_work_and_drift_findings() {
    let sources = build_sources_view(&snapshot());
    let pinned = source(&sources, "core-repo");
    assert_eq!(pinned.status, SourceHealthState::Pinned);
    assert_eq!(pinned.health, StatusState::Ok);
    assert_eq!(pinned.pin.as_deref(), Some("commit:fixture-head"));
    assert_eq!(pinned.trust_level, "project-source");
    assert!(pinned.visible_roles.contains(&"host".to_owned()));
    assert!(pinned.related_work_orders.contains(&"WO-0242".to_owned()));
    assert_eq!(pinned.detail.freshness, "pinned");
    assert!(pinned.detail.next_action.is_none());

    let stale = source(&sources, "readme-console-mock");
    assert_eq!(stale.status, SourceHealthState::Stale);
    assert_eq!(stale.health, StatusState::Warn);
    assert_eq!(stale.pin.as_deref(), Some("sha256:old-readme-console-mock"));
    assert!(stale
        .findings
        .iter()
        .any(|finding| finding.contains("regenerated")));
    assert!(stale
        .detail
        .next_action
        .as_deref()
        .is_some_and(|action| action.contains("refreshing source pin")));

    let mut drift_snapshot = snapshot();
    drift_snapshot.sources.extend([
        source_row("missing-doc", SourceHealthState::Missing, None),
        source_row(
            "trust-changed-api",
            SourceHealthState::TrustChanged,
            Some("commit:new-trust"),
        ),
        source_row(
            "visibility-denied-policy",
            SourceHealthState::VisibilityDenied,
            Some("sha256:policy"),
        ),
    ]);
    let drift = build_sources_view(&drift_snapshot);
    assert_eq!(source(&drift, "missing-doc").health, StatusState::Blocking);
    assert_eq!(
        source(&drift, "trust-changed-api").detail.freshness,
        "trust-changed"
    );
    assert!(source(&drift, "visibility-denied-policy")
        .detail
        .next_action
        .as_deref()
        .is_some_and(|action| action.contains("role visibility")));
}

fn gate<'a>(rows: &'a [GateProgressView], work_order: &str) -> &'a GateProgressView {
    rows.iter()
        .find(|row| row.work_order == work_order)
        .expect("gate row")
}

fn evidence_row<'a>(rows: &'a [EvidenceClosureView], work_order: &str) -> &'a EvidenceClosureView {
    rows.iter()
        .find(|row| row.work_order == work_order)
        .expect("evidence row")
}

fn source<'a>(rows: &'a [SourceHealthView], source_id: &str) -> &'a SourceHealthView {
    rows.iter()
        .find(|row| row.source_id == source_id)
        .expect("source row")
}

fn source_row(
    source_id: &str,
    status: SourceHealthState,
    pin: Option<&str>,
) -> SourceHealthSnapshot {
    SourceHealthSnapshot {
        source_id: source_id.to_owned(),
        status,
        pin: pin.map(str::to_owned),
        trust_level: "external-doc".to_owned(),
        visible_roles: vec!["host".to_owned(), "reviewer".to_owned()],
        findings: vec![format!("{source_id} finding")],
        related_work_orders: vec!["WO-0242".to_owned()],
    }
}
