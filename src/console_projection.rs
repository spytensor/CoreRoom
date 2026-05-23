//! Projection helpers from existing CoreRoom facts into console snapshot rows.
//!
//! These helpers preserve the evidence-bearing fields needed by `@host` for
//! next-action decisions. They do not poll GitHub, render UI, or infer
//! completion from prose.

use crate::console_snapshot::{
    EvidenceClosureState, EvidenceSnapshot, GateSnapshot, SourceHealthSnapshot, SourceHealthState,
    StatusState, WorkLifecycle, WorkSnapshot,
};
use crate::evidence_packet::{EvidencePacket, EvidenceStatus};
use crate::github_status::{
    derive_github_work_order_status, CheckState, EvidencePacketState, GitHubWorkOrderFacts,
    WorkOrderLifecycle,
};
use crate::source_graph::{SourceGraphFinding, SourceGraphFindingKind};
use crate::source_registry::SourceTrustLevel;
use crate::work_order::WorkOrder;

/// Project a WorkOrder and GitHub facts into a console Work row.
#[must_use]
pub fn project_work_snapshot(
    work_order: &WorkOrder,
    github: &GitHubWorkOrderFacts,
    source_citations: Vec<String>,
) -> WorkSnapshot {
    let status = derive_github_work_order_status(github);
    WorkSnapshot {
        id: work_order.id.clone(),
        title: work_order.title.clone(),
        phase: work_order.phase.clone(),
        epic: work_order.epic.clone(),
        github_issue: work_order.github_issue.or(Some(github.issue)),
        branch: work_order.branch.clone().or_else(|| github.branch.clone()),
        pull_request: work_order
            .pull_request
            .or_else(|| github.pull_request.as_ref().map(|pr| pr.number)),
        ci_state: ci_state(
            &github
                .checks
                .iter()
                .map(|check| check.state)
                .collect::<Vec<_>>(),
        ),
        evidence_state: evidence_state(github.evidence),
        tracker_state: tracker_state(github, status.lifecycle),
        lifecycle: status.lifecycle.into(),
        source_citations,
    }
}

/// Project gate facts into a console gate row.
#[must_use]
pub fn project_gate_snapshot(input: GateProjectionInput) -> GateSnapshot {
    GateSnapshot {
        work_order: input.work_order,
        current_phase: input.current_phase,
        blocked_reason: input.blocked_reason,
        missing_reviews: input.missing_reviews,
        stale_plan_sha: input.stale_plan_sha,
        signoff_ready: input.signoff_ready,
    }
}

/// Input facts for a gate row projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateProjectionInput {
    /// WorkOrder id.
    pub work_order: String,
    /// Current phase.
    pub current_phase: String,
    /// Blocked reason.
    pub blocked_reason: Option<String>,
    /// Missing reviews.
    pub missing_reviews: Vec<String>,
    /// Stale plan SHA.
    pub stale_plan_sha: Option<String>,
    /// Signoff readiness.
    pub signoff_ready: bool,
}

/// Project an optional Evidence Packet into a console evidence row.
#[must_use]
pub fn project_evidence_snapshot(
    work_order: &str,
    packet: Option<&EvidencePacket>,
) -> EvidenceSnapshot {
    let Some(packet) = packet else {
        return EvidenceSnapshot {
            work_order: work_order.to_owned(),
            status: EvidenceClosureState::Missing,
            missing_fields: vec!["Evidence Packet".to_owned()],
            unverified_items: Vec::new(),
            rollback: None,
            tracker_updated: false,
        };
    };
    let report = packet.completion_report().ok();
    EvidenceSnapshot {
        work_order: packet.work_order.clone(),
        status: match (
            packet.status,
            report.as_ref().map(|report| report.missing.is_empty()),
        ) {
            (EvidenceStatus::Complete, Some(true)) => EvidenceClosureState::Complete,
            (_, Some(false)) => EvidenceClosureState::Incomplete,
            _ => EvidenceClosureState::Unverified,
        },
        missing_fields: report
            .as_ref()
            .map_or_else(Vec::new, |report| report.missing.clone()),
        unverified_items: packet.unverified_items.clone(),
        rollback: Some(packet.rollback.clone()),
        tracker_updated: packet.tracker_update.checkbox_updated
            && packet.tracker_update.evidence_ledger_updated,
    }
}

/// Project a source graph finding into a console source health row.
#[must_use]
pub fn project_source_health_snapshot(
    finding: &SourceGraphFinding,
    trust_level: SourceTrustLevel,
    visible_roles: Vec<String>,
    related_work_orders: Vec<String>,
) -> SourceHealthSnapshot {
    SourceHealthSnapshot {
        source_id: finding.source_id.clone(),
        status: source_health_state(finding.kind),
        trust_level: trust_level.label().to_owned(),
        visible_roles,
        findings: vec![finding.message.clone()],
        related_work_orders,
    }
}

fn ci_state(checks: &[CheckState]) -> StatusState {
    if checks.is_empty() {
        return StatusState::Unknown;
    }
    if checks
        .iter()
        .any(|state| matches!(state, CheckState::Fail | CheckState::Cancelled))
    {
        StatusState::Blocking
    } else if checks
        .iter()
        .any(|state| matches!(state, CheckState::Pending | CheckState::Unknown))
    {
        StatusState::Warn
    } else {
        StatusState::Ok
    }
}

fn evidence_state(state: EvidencePacketState) -> StatusState {
    match state {
        EvidencePacketState::Complete => StatusState::Ok,
        EvidencePacketState::Incomplete => StatusState::Warn,
        EvidencePacketState::Missing => StatusState::Blocking,
    }
}

fn tracker_state(github: &GitHubWorkOrderFacts, lifecycle: WorkOrderLifecycle) -> StatusState {
    if github.tracker.checkbox_checked
        && github.tracker.ledger_status.as_deref() == Some("merged")
        && github.tracker.ledger_tracker_updated == Some(true)
    {
        StatusState::Ok
    } else if lifecycle == WorkOrderLifecycle::MergedTrackerStale {
        StatusState::Blocking
    } else {
        StatusState::Warn
    }
}

fn source_health_state(kind: SourceGraphFindingKind) -> SourceHealthState {
    match kind {
        SourceGraphFindingKind::MissingSource => SourceHealthState::Missing,
        SourceGraphFindingKind::TrustChanged => SourceHealthState::TrustChanged,
        SourceGraphFindingKind::VisibilityDenied | SourceGraphFindingKind::VisibilityChanged => {
            SourceHealthState::VisibilityDenied
        }
        SourceGraphFindingKind::CommitChanged
        | SourceGraphFindingKind::FileHashChanged
        | SourceGraphFindingKind::UrlSnapshotStale
        | SourceGraphFindingKind::PinChanged => SourceHealthState::Stale,
    }
}

impl From<WorkOrderLifecycle> for WorkLifecycle {
    fn from(value: WorkOrderLifecycle) -> Self {
        match value {
            WorkOrderLifecycle::NotStarted => Self::NotStarted,
            WorkOrderLifecycle::Ready => Self::Ready,
            WorkOrderLifecycle::InProgress => Self::InProgress,
            WorkOrderLifecycle::InReview => Self::InReview,
            WorkOrderLifecycle::FailedCi => Self::FailedCi,
            WorkOrderLifecycle::Blocked => Self::Blocked,
            WorkOrderLifecycle::MergedTrackerStale => Self::MergedTrackerStale,
            WorkOrderLifecycle::Closed => Self::Closed,
        }
    }
}
