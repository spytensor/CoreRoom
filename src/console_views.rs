//! Detailed console views for role lanes and WorkOrders.
//!
//! These models are still read-only projections over `CoreRoomSnapshot`. They
//! give the future navigator concrete data to display without re-reading logs
//! or inventing ownership.

use crate::console_snapshot::{
    CoreRoomSnapshot, EvidenceClosureState, RoleLaneState, StatusState, WorkLifecycle,
};

/// One role row in the Roles view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleLaneView {
    /// Role name without leading `@`.
    pub role: String,
    /// Whether the role is enabled.
    pub enabled: bool,
    /// Backing engine id.
    pub engine: String,
    /// Model label when known.
    pub model: Option<String>,
    /// Role lane state.
    pub state: RoleLaneState,
    /// Permission mode summary.
    pub permission_mode: Option<String>,
    /// WorkOrder currently associated with the role.
    pub current_work_order: Option<String>,
    /// Current gate phase when known.
    pub current_gate_phase: Option<String>,
    /// Compact health status for table styling.
    pub status: StatusState,
    /// Recommended next action when attention is needed.
    pub next_action: Option<String>,
}

/// WorkOrder row for the WorkOrders view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkOrderView {
    /// WorkOrder id.
    pub id: String,
    /// Human title.
    pub title: String,
    /// Lifecycle state.
    pub lifecycle: WorkLifecycle,
    /// Bound GitHub Issue.
    pub github_issue: Option<u64>,
    /// Branch name.
    pub branch: Option<String>,
    /// Pull request number.
    pub pull_request: Option<u64>,
    /// CI status.
    pub ci_state: StatusState,
    /// Evidence status.
    pub evidence_state: StatusState,
    /// Tracker status.
    pub tracker_state: StatusState,
    /// Roles currently associated with this work item.
    pub owner_roles: Vec<String>,
    /// Source/citation labels from the snapshot.
    pub citations: Vec<String>,
    /// Detail panel text.
    pub detail: WorkOrderDetail,
}

/// Detail panel for a WorkOrder row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkOrderDetail {
    /// Blocking reason when known.
    pub blocker: Option<String>,
    /// Next action derived from lifecycle and closure state.
    pub next_action: Option<String>,
    /// Whether the row is ready for closure.
    pub closure_ready: bool,
}

/// Build the Roles view from snapshot role lanes.
#[must_use]
pub fn build_roles_view(snapshot: &CoreRoomSnapshot) -> Vec<RoleLaneView> {
    snapshot
        .runtime
        .roles
        .iter()
        .map(|role| RoleLaneView {
            role: role.role.clone(),
            enabled: role.enabled,
            engine: role.engine.clone(),
            model: role.model.clone(),
            state: role.state,
            permission_mode: role.permission_mode.clone(),
            current_work_order: role.current_work_order.clone(),
            current_gate_phase: role.current_gate_phase.clone(),
            status: role_status(
                role.state,
                role.waiting_approval,
                role.permission_mode.as_deref(),
            ),
            next_action: role_next_action(
                role.state,
                role.waiting_approval,
                role.permission_mode.as_deref(),
            ),
        })
        .collect()
}

/// Build the WorkOrders view from snapshot work, role, gate, and evidence facts.
#[must_use]
pub fn build_workorders_view(snapshot: &CoreRoomSnapshot) -> Vec<WorkOrderView> {
    snapshot
        .work
        .iter()
        .map(|work| {
            let owner_roles = snapshot
                .runtime
                .roles
                .iter()
                .filter(|role| role.current_work_order.as_deref() == Some(work.id.as_str()))
                .map(|role| format!("@{}", role.role))
                .collect::<Vec<_>>();
            let gate = snapshot
                .gates
                .iter()
                .find(|gate| gate.work_order == work.id);
            let evidence = snapshot
                .evidence
                .iter()
                .find(|evidence| evidence.work_order == work.id);
            let detail = work_detail(
                work.lifecycle,
                work.ci_state,
                work.evidence_state,
                work.tracker_state,
                gate.and_then(|gate| gate.blocked_reason.clone()),
                evidence.map(|evidence| evidence.status),
            );
            WorkOrderView {
                id: work.id.clone(),
                title: work.title.clone(),
                lifecycle: work.lifecycle,
                github_issue: work.github_issue,
                branch: work.branch.clone(),
                pull_request: work.pull_request,
                ci_state: work.ci_state,
                evidence_state: work.evidence_state,
                tracker_state: work.tracker_state,
                owner_roles,
                citations: work.source_citations.clone(),
                detail,
            }
        })
        .collect()
}

fn role_status(
    state: RoleLaneState,
    waiting_approval: bool,
    permission_mode: Option<&str>,
) -> StatusState {
    if matches!(state, RoleLaneState::Blocked) {
        StatusState::Blocking
    } else if waiting_approval
        || matches!(
            state,
            RoleLaneState::WaitingApproval
                | RoleLaneState::WaitingUser
                | RoleLaneState::StaleSession
        )
        || permission_mode == Some("bypass")
    {
        StatusState::Warn
    } else {
        StatusState::Ok
    }
}

fn role_next_action(
    state: RoleLaneState,
    waiting_approval: bool,
    permission_mode: Option<&str>,
) -> Option<String> {
    if matches!(state, RoleLaneState::Blocked) {
        Some("resolve role blocker before relying on output".to_owned())
    } else if waiting_approval || matches!(state, RoleLaneState::WaitingApproval) {
        Some("ask user to approve, deny, or revise the request".to_owned())
    } else if matches!(state, RoleLaneState::WaitingUser) {
        Some("wait for user decision".to_owned())
    } else if matches!(state, RoleLaneState::StaleSession) {
        Some("refresh role session before using stale context".to_owned())
    } else if permission_mode == Some("bypass") {
        Some("confirm bypass remains intentional".to_owned())
    } else {
        None
    }
}

fn work_detail(
    lifecycle: WorkLifecycle,
    ci_state: StatusState,
    evidence_state: StatusState,
    tracker_state: StatusState,
    gate_blocker: Option<String>,
    evidence_status: Option<EvidenceClosureState>,
) -> WorkOrderDetail {
    let blocker = gate_blocker.or_else(|| lifecycle_blocker(lifecycle, ci_state));
    let closure_ready = lifecycle == WorkLifecycle::Closed
        && ci_state == StatusState::Ok
        && evidence_state == StatusState::Ok
        && tracker_state == StatusState::Ok
        && matches!(evidence_status, None | Some(EvidenceClosureState::Complete));
    let next_action = if let Some(blocker) = &blocker {
        Some(format!("resolve blocker: {blocker}"))
    } else if lifecycle == WorkLifecycle::FailedCi || ci_state == StatusState::Blocking {
        Some("inspect and fix failing CI before review".to_owned())
    } else if evidence_state != StatusState::Ok || tracker_state != StatusState::Ok {
        Some("complete evidence and tracker closure before claiming done".to_owned())
    } else if matches!(lifecycle, WorkLifecycle::InReview) {
        Some("wait for review/CI, then merge and update tracker".to_owned())
    } else if closure_ready {
        None
    } else {
        Some("advance WorkOrder through the next gate phase".to_owned())
    };
    WorkOrderDetail {
        blocker,
        next_action,
        closure_ready,
    }
}

fn lifecycle_blocker(lifecycle: WorkLifecycle, ci_state: StatusState) -> Option<String> {
    match lifecycle {
        WorkLifecycle::Blocked => Some("work lifecycle is blocked".to_owned()),
        WorkLifecycle::FailedCi => Some("work lifecycle has failed CI".to_owned()),
        WorkLifecycle::MergedTrackerStale => {
            Some("merged work has stale tracker evidence".to_owned())
        }
        _ if ci_state == StatusState::Blocking => Some("CI state is blocking".to_owned()),
        _ => None,
    }
}
