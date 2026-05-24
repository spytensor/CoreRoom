//! Detailed role and WorkOrder console view fixtures.

use std::collections::HashSet;

use coreroom::console_snapshot::{CoreRoomSnapshot, StatusState, WorkLifecycle};
use coreroom::console_views::{build_roles_view, build_workorders_view, WorkOrderView};

fn snapshot() -> CoreRoomSnapshot {
    toml::from_str(include_str!("fixtures/console_snapshot_v08.toml")).expect("snapshot")
}

#[test]
fn roles_view_covers_enabled_working_blocked_waiting_and_stale_states() {
    let roles = build_roles_view(&snapshot());
    let states = roles
        .iter()
        .map(|role| format!("{:?}", role.state))
        .collect::<HashSet<_>>();

    assert!(states.contains("Idle"));
    assert!(states.contains("Working"));
    assert!(states.contains("Reviewing"));
    assert!(states.contains("Blocked"));
    assert!(states.contains("WaitingApproval"));
    assert!(states.contains("WaitingUser"));
    assert!(states.contains("StaleSession"));

    let security = roles
        .iter()
        .find(|role| role.role == "security")
        .expect("security role");
    assert_eq!(security.status, StatusState::Blocking);
    assert!(security
        .next_action
        .as_deref()
        .is_some_and(|action| { action.contains("resolve role blocker") }));
}

#[test]
fn workorders_view_preserves_issue_branch_pr_status_and_owner_roles() {
    let work = build_workorders_view(&snapshot());
    let row = work_order(&work, "WO-0242");

    assert_eq!(row.github_issue, Some(242));
    assert_eq!(
        row.branch.as_deref(),
        Some("feat/v0.8-242-coreroom-snapshot-schema")
    );
    assert_eq!(row.pull_request, None);
    assert_eq!(row.ci_state, StatusState::Unknown);
    assert_eq!(row.evidence_state, StatusState::Warn);
    assert_eq!(row.tracker_state, StatusState::Warn);
    assert!(row.owner_roles.contains(&"@host".to_owned()));
    assert!(row.owner_roles.contains(&"@reviewer".to_owned()));
    assert!(row.citations.contains(&"tracker:#238".to_owned()));
}

#[test]
fn workorders_view_explains_blocked_failed_ci_in_review_and_closed_work() {
    let blocked = build_workorders_view(&snapshot());
    let blocked_row = work_order(&blocked, "WO-0251");
    assert_eq!(blocked_row.lifecycle, WorkLifecycle::Blocked);
    assert!(blocked_row
        .detail
        .blocker
        .as_deref()
        .is_some_and(|blocker| blocker.contains("Requires snapshot")));
    assert!(blocked_row.detail.next_action.is_some());

    let mut failed_snapshot = snapshot();
    let failed_work = failed_snapshot
        .work
        .iter_mut()
        .find(|work| work.id == "WO-0242")
        .expect("WO-0242");
    failed_work.lifecycle = WorkLifecycle::FailedCi;
    failed_work.ci_state = StatusState::Blocking;
    let failed = build_workorders_view(&failed_snapshot);
    let failed_row = work_order(&failed, "WO-0242");
    assert_eq!(failed_row.lifecycle, WorkLifecycle::FailedCi);
    assert!(failed_row
        .detail
        .blocker
        .as_deref()
        .is_some_and(|blocker| blocker.contains("failed CI")));

    let mut review_snapshot = snapshot();
    let review_work = review_snapshot
        .work
        .iter_mut()
        .find(|work| work.id == "WO-0242")
        .expect("WO-0242");
    review_work.lifecycle = WorkLifecycle::InReview;
    let review = build_workorders_view(&review_snapshot);
    let review_row = work_order(&review, "WO-0242");
    assert_eq!(review_row.lifecycle, WorkLifecycle::InReview);
    assert!(review_row
        .detail
        .next_action
        .as_deref()
        .is_some_and(|action| action.contains("evidence and tracker")));

    let closed = build_workorders_view(&snapshot());
    let closed_row = work_order(&closed, "WO-0241");
    assert_eq!(closed_row.lifecycle, WorkLifecycle::Closed);
    assert!(closed_row.detail.closure_ready);
    assert!(closed_row.detail.next_action.is_none());
}

fn work_order<'a>(rows: &'a [WorkOrderView], id: &str) -> &'a WorkOrderView {
    rows.iter().find(|row| row.id == id).expect("work row")
}
