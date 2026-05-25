//! Detailed console views for role lanes and WorkOrders.
//!
//! These models are still read-only projections over `CoreRoomSnapshot`. They
//! give the future navigator concrete data to display without re-reading logs
//! or inventing ownership.

use crate::config::{AuthorityScope, RoleAccess};
use crate::console_snapshot::{
    CoreRoomSnapshot, EvidenceClosureState, InternalDelegationState, RoleLaneState,
    SourceHealthSnapshot, SourceHealthState, StatusState, WorkLifecycle,
};
use crate::crep::CrepEvent;
use crate::work_order::{WorkOrderRoleAccess, WorkOrderRoleGrant};

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
    /// Configured access class when explicitly set on the role.
    pub configured_access: Option<RoleAccess>,
    /// Effective access class after applying host/engineer defaults.
    pub effective_access: Option<RoleAccess>,
    /// Human owner for role priors/authority.
    pub owner: Option<String>,
    /// Domain authority scopes where this role may issue plan vetoes.
    pub authority: Vec<AuthorityScope>,
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
    /// Structural role grants scoped to this WorkOrder.
    pub role_grants: Vec<WorkOrderRoleGrant>,
    /// Roles with structural write grants for this WorkOrder.
    pub escalated_roles: Vec<String>,
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

/// One row in the Gates view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateProgressView {
    /// WorkOrder id.
    pub work_order: String,
    /// Current gate phase.
    pub current_phase: String,
    /// Blocked reason when present.
    pub blocked_reason: Option<String>,
    /// Required reviews that are still missing.
    pub missing_reviews: Vec<String>,
    /// Stale plan hash when detected.
    pub stale_plan_sha: Option<String>,
    /// Whether the gate can proceed to signoff.
    pub signoff_ready: bool,
    /// Compact health status for table styling.
    pub status: StatusState,
    /// Source citations inherited from the bound WorkOrder.
    pub citations: Vec<String>,
    /// Detail panel text.
    pub detail: GateDetailView,
}

/// Detail panel for a gate row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateDetailView {
    /// Human-readable freshness marker.
    pub freshness: String,
    /// Recommended next action.
    pub next_action: Option<String>,
}

/// One row in the Evidence view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceClosureView {
    /// WorkOrder id.
    pub work_order: String,
    /// Evidence packet status.
    pub status: EvidenceClosureState,
    /// Missing evidence fields.
    pub missing_fields: Vec<String>,
    /// Explicitly unverified items.
    pub unverified_items: Vec<String>,
    /// Rollback plan.
    pub rollback: Option<String>,
    /// Whether the tracker row/evidence ledger is updated.
    pub tracker_updated: bool,
    /// Compact health status for table styling.
    pub health: StatusState,
    /// Source citations inherited from the bound WorkOrder.
    pub citations: Vec<String>,
    /// Detail panel text.
    pub detail: EvidenceDetailView,
}

/// Detail panel for an evidence row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceDetailView {
    /// Human-readable freshness marker.
    pub freshness: String,
    /// Recommended next action.
    pub next_action: Option<String>,
    /// Whether evidence can support a completion claim.
    pub closure_ready: bool,
}

/// One row in the Sources view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceHealthView {
    /// Source Registry id.
    pub source_id: String,
    /// Source health status.
    pub status: SourceHealthState,
    /// Pinned source version when observed.
    pub pin: Option<String>,
    /// Trust level label.
    pub trust_level: String,
    /// Roles that may see this source.
    pub visible_roles: Vec<String>,
    /// Source findings.
    pub findings: Vec<String>,
    /// Related WorkOrders.
    pub related_work_orders: Vec<String>,
    /// Compact health status for table styling.
    pub health: StatusState,
    /// Detail panel text.
    pub detail: SourceDetailView,
}

/// Detail panel for a source row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDetailView {
    /// Human-readable freshness marker.
    pub freshness: String,
    /// Recommended next action.
    pub next_action: Option<String>,
}

/// Filter for the CREP logs view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CrepLogFilter {
    /// Restrict to one role.
    pub role: Option<String>,
    /// Restrict to one thread id.
    pub thread_id: Option<String>,
    /// Restrict to one turn id.
    pub turn_id: Option<String>,
    /// Restrict to event type labels such as `role_spoke`.
    pub event_types: Vec<String>,
}

/// One row in the CREP logs view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrepLogRow {
    /// Event type label.
    pub event_type: String,
    /// Role if the event is role-scoped.
    pub role: Option<String>,
    /// Turn id if present.
    pub turn_id: Option<String>,
    /// Thread id if present.
    pub thread_id: Option<String>,
    /// Compact event summary.
    pub summary: String,
    /// Whether this belongs to internal/delegation/audit context.
    pub internal: bool,
    /// Compact health status for table styling.
    pub status: StatusState,
}

/// One internal delegation row for Xray views.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InternalDelegationView {
    /// Delegated role.
    pub role: String,
    /// Related WorkOrder.
    pub work_order: Option<String>,
    /// Delegation state.
    pub state: InternalDelegationState,
    /// Compact summary.
    pub summary: String,
    /// Xray/log reference.
    pub xray_ref: Option<String>,
}

/// WorkOrder Xray row showing the engineering evidence chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkOrderXrayView {
    /// WorkOrder id.
    pub work_order: String,
    /// WorkOrder title.
    pub title: String,
    /// Xray chain steps.
    pub steps: Vec<XrayStep>,
    /// Source/citation labels from the WorkOrder and related sources.
    pub citations: Vec<String>,
    /// Internal delegations related to this WorkOrder.
    pub internal_delegations: Vec<InternalDelegationView>,
    /// Human-readable freshness marker.
    pub freshness: String,
    /// Whether the WorkOrder chain supports closure.
    pub closure_ready: bool,
}

/// One step in the WorkOrder Xray chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XrayStep {
    /// Step name such as `issue`, `branch`, or `evidence`.
    pub name: String,
    /// Display value.
    pub value: String,
    /// Compact status.
    pub status: StatusState,
    /// Freshness marker for this step.
    pub freshness: String,
    /// Citations proving this step.
    pub citations: Vec<String>,
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
            configured_access: role.configured_access,
            effective_access: role.effective_access,
            owner: role.owner.clone(),
            authority: role.authority.clone(),
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
            let escalated_roles = work
                .role_grants
                .iter()
                .filter(|grant| grant.access == WorkOrderRoleAccess::Write)
                .map(|grant| format!("@{}", grant.role))
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
                role_grants: work.role_grants.clone(),
                escalated_roles,
                citations: work.source_citations.clone(),
                detail,
            }
        })
        .collect()
}

/// Build the Gates view from snapshot gate facts.
#[must_use]
pub fn build_gates_view(snapshot: &CoreRoomSnapshot) -> Vec<GateProgressView> {
    snapshot
        .gates
        .iter()
        .map(|gate| {
            let citations = work_citations(snapshot, &gate.work_order);
            let status = gate_status(
                gate.blocked_reason.as_deref(),
                &gate.missing_reviews,
                gate.stale_plan_sha.as_deref(),
                gate.signoff_ready,
            );
            GateProgressView {
                work_order: gate.work_order.clone(),
                current_phase: gate.current_phase.clone(),
                blocked_reason: gate.blocked_reason.clone(),
                missing_reviews: gate.missing_reviews.clone(),
                stale_plan_sha: gate.stale_plan_sha.clone(),
                signoff_ready: gate.signoff_ready,
                status,
                citations,
                detail: GateDetailView {
                    freshness: gate_freshness(gate.stale_plan_sha.as_deref()).to_owned(),
                    next_action: gate_next_action(
                        gate.blocked_reason.as_deref(),
                        &gate.missing_reviews,
                        gate.stale_plan_sha.as_deref(),
                        gate.signoff_ready,
                    ),
                },
            }
        })
        .collect()
}

/// Build the Evidence view from snapshot evidence facts.
#[must_use]
pub fn build_evidence_view(snapshot: &CoreRoomSnapshot) -> Vec<EvidenceClosureView> {
    snapshot
        .evidence
        .iter()
        .map(|evidence| {
            let health = evidence_health(evidence.status, evidence.tracker_updated);
            let closure_ready =
                evidence.status == EvidenceClosureState::Complete && evidence.tracker_updated;
            EvidenceClosureView {
                work_order: evidence.work_order.clone(),
                status: evidence.status,
                missing_fields: evidence.missing_fields.clone(),
                unverified_items: evidence.unverified_items.clone(),
                rollback: evidence.rollback.clone(),
                tracker_updated: evidence.tracker_updated,
                health,
                citations: work_citations(snapshot, &evidence.work_order),
                detail: EvidenceDetailView {
                    freshness: evidence_freshness(evidence.status, evidence.tracker_updated)
                        .to_owned(),
                    next_action: evidence_next_action(evidence),
                    closure_ready,
                },
            }
        })
        .collect()
}

/// Build the Sources view from snapshot source health facts.
#[must_use]
pub fn build_sources_view(snapshot: &CoreRoomSnapshot) -> Vec<SourceHealthView> {
    snapshot
        .sources
        .iter()
        .map(|source| SourceHealthView {
            source_id: source.source_id.clone(),
            status: source.status,
            pin: source.pin.clone(),
            trust_level: source.trust_level.clone(),
            visible_roles: source.visible_roles.clone(),
            findings: source.findings.clone(),
            related_work_orders: source.related_work_orders.clone(),
            health: source_status(source.status),
            detail: SourceDetailView {
                freshness: source_freshness(source.status).to_owned(),
                next_action: source_next_action(source.status, source.pin.as_deref()),
            },
        })
        .collect()
}

/// Build a filtered CREP logs view.
#[must_use]
pub fn build_crep_logs_view(events: &[CrepEvent], filter: &CrepLogFilter) -> Vec<CrepLogRow> {
    events
        .iter()
        .map(crep_log_row)
        .filter(|row| crep_log_matches(row, filter))
        .collect()
}

/// Build one WorkOrder Xray view.
#[must_use]
pub fn build_workorder_xray_view(
    snapshot: &CoreRoomSnapshot,
    work_order: &str,
) -> Option<WorkOrderXrayView> {
    let work = snapshot.work.iter().find(|work| work.id == work_order)?;
    let evidence = snapshot
        .evidence
        .iter()
        .find(|evidence| evidence.work_order == work.id);
    let related_sources = snapshot
        .sources
        .iter()
        .filter(|source| source.related_work_orders.iter().any(|id| id == &work.id))
        .collect::<Vec<_>>();
    let source_status = related_source_status(&related_sources);
    let source_citations = related_sources
        .iter()
        .map(|source| {
            let pin = source.pin.as_deref().unwrap_or("pin:not-observed");
            format!("source:{}@{pin}", source.source_id)
        })
        .collect::<Vec<_>>();
    let citations = work
        .source_citations
        .iter()
        .cloned()
        .chain(source_citations)
        .collect::<Vec<_>>();
    let internal_delegations = snapshot
        .conversation
        .internal_activity
        .iter()
        .filter(|activity| activity.work_order.as_deref() == Some(work.id.as_str()))
        .map(|activity| InternalDelegationView {
            role: activity.role.clone(),
            work_order: activity.work_order.clone(),
            state: activity.state,
            summary: activity.summary.clone(),
            xray_ref: activity.xray_ref.clone(),
        })
        .collect::<Vec<_>>();
    let closure_ready = work.lifecycle == WorkLifecycle::Closed
        && work.ci_state == StatusState::Ok
        && work.evidence_state == StatusState::Ok
        && work.tracker_state == StatusState::Ok
        && evidence.is_none_or(|evidence| {
            evidence.status == EvidenceClosureState::Complete && evidence.tracker_updated
        });
    Some(WorkOrderXrayView {
        work_order: work.id.clone(),
        title: work.title.clone(),
        steps: vec![
            xray_step(
                "workorder",
                work.id.clone(),
                status_for_lifecycle(work.lifecycle),
                lifecycle_freshness(work.lifecycle),
                vec![format!("work:{}", work.id)],
            ),
            xray_step(
                "issue",
                work.github_issue
                    .map_or_else(|| "missing".to_owned(), |issue| format!("#{issue}")),
                presence_status(work.github_issue.is_some()),
                presence_freshness(work.github_issue.is_some()),
                work.github_issue
                    .map_or_else(Vec::new, |issue| vec![format!("issue:#{issue}")]),
            ),
            xray_step(
                "branch",
                work.branch.clone().unwrap_or_else(|| "missing".to_owned()),
                presence_status(work.branch.is_some()),
                presence_freshness(work.branch.is_some()),
                work.branch
                    .as_ref()
                    .map_or_else(Vec::new, |branch| vec![format!("branch:{branch}")]),
            ),
            xray_step(
                "pr",
                work.pull_request
                    .map_or_else(|| "missing".to_owned(), |pr| format!("#{pr}")),
                presence_status(work.pull_request.is_some()),
                presence_freshness(work.pull_request.is_some()),
                work.pull_request
                    .map_or_else(Vec::new, |pr| vec![format!("pr:#{pr}")]),
            ),
            xray_step(
                "ci",
                status_label(work.ci_state).to_owned(),
                work.ci_state,
                status_freshness(work.ci_state),
                Vec::new(),
            ),
            xray_step(
                "evidence",
                evidence.map_or_else(
                    || status_label(work.evidence_state).to_owned(),
                    |evidence| evidence_status_label(evidence.status).to_owned(),
                ),
                work.evidence_state,
                evidence.map_or_else(
                    || status_freshness(work.evidence_state),
                    |evidence| evidence_freshness(evidence.status, evidence.tracker_updated),
                ),
                vec![format!("evidence:{}", work.id)],
            ),
            xray_step(
                "tracker",
                status_label(work.tracker_state).to_owned(),
                work.tracker_state,
                status_freshness(work.tracker_state),
                vec![format!("tracker:#{}", snapshot.project.tracker_issue)],
            ),
            xray_step(
                "sources",
                source_count_label(related_sources.len()),
                source_status,
                status_freshness(source_status),
                citations.clone(),
            ),
        ],
        citations,
        internal_delegations,
        freshness: xray_freshness(work.lifecycle, work.tracker_state, source_status).to_owned(),
        closure_ready,
    })
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

fn work_citations(snapshot: &CoreRoomSnapshot, work_order: &str) -> Vec<String> {
    snapshot
        .work
        .iter()
        .find(|work| work.id == work_order)
        .map_or_else(Vec::new, |work| work.source_citations.clone())
}

fn gate_status(
    blocked_reason: Option<&str>,
    missing_reviews: &[String],
    stale_plan_sha: Option<&str>,
    signoff_ready: bool,
) -> StatusState {
    if blocked_reason.is_some() {
        StatusState::Blocking
    } else if !missing_reviews.is_empty() || stale_plan_sha.is_some() || !signoff_ready {
        StatusState::Warn
    } else {
        StatusState::Ok
    }
}

fn gate_freshness(stale_plan_sha: Option<&str>) -> &'static str {
    if stale_plan_sha.is_some() {
        "stale-plan"
    } else {
        "fresh"
    }
}

fn gate_next_action(
    blocked_reason: Option<&str>,
    missing_reviews: &[String],
    stale_plan_sha: Option<&str>,
    signoff_ready: bool,
) -> Option<String> {
    if let Some(reason) = blocked_reason {
        Some(format!("resolve blocked gate: {reason}"))
    } else if !missing_reviews.is_empty() {
        Some(format!("collect reviews: {}", missing_reviews.join(", ")))
    } else if let Some(plan_sha) = stale_plan_sha {
        Some(format!("refresh stale plan before signoff: {plan_sha}"))
    } else if !signoff_ready {
        Some("finish review/signoff evidence before implementation".to_owned())
    } else {
        None
    }
}

fn evidence_health(status: EvidenceClosureState, tracker_updated: bool) -> StatusState {
    if !tracker_updated {
        return StatusState::Blocking;
    }
    match status {
        EvidenceClosureState::Complete => StatusState::Ok,
        EvidenceClosureState::Incomplete | EvidenceClosureState::Unverified => StatusState::Warn,
        EvidenceClosureState::Missing => StatusState::Blocking,
    }
}

fn evidence_freshness(status: EvidenceClosureState, tracker_updated: bool) -> &'static str {
    match (status, tracker_updated) {
        (EvidenceClosureState::Complete, true) => "complete",
        (EvidenceClosureState::Complete, false) => "tracker-stale",
        (EvidenceClosureState::Incomplete, _) => "incomplete",
        (EvidenceClosureState::Missing, _) => "missing",
        (EvidenceClosureState::Unverified, _) => "unverified",
    }
}

fn evidence_next_action(evidence: &crate::console_snapshot::EvidenceSnapshot) -> Option<String> {
    if evidence.status == EvidenceClosureState::Complete && evidence.tracker_updated {
        None
    } else if !evidence.missing_fields.is_empty() {
        Some(format!(
            "fill evidence: {}",
            evidence.missing_fields.join(", ")
        ))
    } else if !evidence.unverified_items.is_empty() {
        Some(format!(
            "verify items: {}",
            evidence.unverified_items.join(", ")
        ))
    } else if !evidence.tracker_updated {
        Some("update tracker checkbox and Evidence Ledger".to_owned())
    } else {
        Some("complete Evidence Packet before claiming done".to_owned())
    }
}

fn source_status(status: SourceHealthState) -> StatusState {
    match status {
        SourceHealthState::Pinned => StatusState::Ok,
        SourceHealthState::Stale => StatusState::Warn,
        SourceHealthState::Missing
        | SourceHealthState::TrustChanged
        | SourceHealthState::VisibilityDenied => StatusState::Blocking,
    }
}

fn source_freshness(status: SourceHealthState) -> &'static str {
    match status {
        SourceHealthState::Pinned => "pinned",
        SourceHealthState::Stale => "stale",
        SourceHealthState::Missing => "missing",
        SourceHealthState::TrustChanged => "trust-changed",
        SourceHealthState::VisibilityDenied => "visibility-denied",
    }
}

fn source_next_action(status: SourceHealthState, pin: Option<&str>) -> Option<String> {
    match status {
        SourceHealthState::Pinned if pin.is_none() => Some("record source pin".to_owned()),
        SourceHealthState::Pinned => None,
        SourceHealthState::Stale => Some("ask user before refreshing source pin".to_owned()),
        SourceHealthState::Missing => Some("restore or remove missing source".to_owned()),
        SourceHealthState::TrustChanged => Some("confirm trust-level change with user".to_owned()),
        SourceHealthState::VisibilityDenied => {
            Some("fix role visibility before using this source".to_owned())
        }
    }
}

fn crep_log_row(event: &CrepEvent) -> CrepLogRow {
    let event_type = crep_event_type(event).to_owned();
    let role = crep_event_role(event).map(str::to_owned);
    let turn_id = crep_event_turn(event).map(str::to_owned);
    let thread_id = crep_event_thread(event).map(str::to_owned);
    CrepLogRow {
        event_type,
        role,
        turn_id,
        thread_id,
        summary: crep_event_summary(event),
        internal: crep_event_internal(event),
        status: crep_event_status(event),
    }
}

fn crep_log_matches(row: &CrepLogRow, filter: &CrepLogFilter) -> bool {
    filter
        .role
        .as_ref()
        .is_none_or(|role| row.role.as_ref() == Some(role))
        && filter
            .thread_id
            .as_ref()
            .is_none_or(|thread_id| row.thread_id.as_ref() == Some(thread_id))
        && filter
            .turn_id
            .as_ref()
            .is_none_or(|turn_id| row.turn_id.as_ref() == Some(turn_id))
        && (filter.event_types.is_empty()
            || filter
                .event_types
                .iter()
                .any(|event_type| event_type == &row.event_type))
}

fn crep_event_type(event: &CrepEvent) -> &'static str {
    match event {
        CrepEvent::RoleStarted { .. } => "role_started",
        CrepEvent::RoleSessionUpdated { .. } => "role_session_updated",
        CrepEvent::TurnDispatched { .. } => "turn_dispatched",
        CrepEvent::WorkTitle { .. } => "work_title",
        CrepEvent::RoleSpoke { .. } => "role_spoke",
        CrepEvent::PhaseAdvanced { .. } => "phase_advanced",
        CrepEvent::PhaseBlocked { .. } => "phase_blocked",
        CrepEvent::PlanReviewed { .. } => "plan_reviewed",
        CrepEvent::PlanOverridden { .. } => "plan_overridden",
        CrepEvent::RoleOutputDelta { .. } => "role_output_delta",
        CrepEvent::TurnInterrupted { .. } => "turn_interrupted",
        CrepEvent::ToolCallProposed { .. } => "tool_call_proposed",
        CrepEvent::ToolCallExecuted { .. } => "tool_call_executed",
        CrepEvent::PermissionDenied { .. } => "permission_denied",
        CrepEvent::RoleStopped { .. } => "role_stopped",
    }
}

fn crep_event_role(event: &CrepEvent) -> Option<&str> {
    match event {
        CrepEvent::RoleStarted { role, .. }
        | CrepEvent::RoleSessionUpdated { role, .. }
        | CrepEvent::TurnDispatched { role, .. }
        | CrepEvent::WorkTitle { role, .. }
        | CrepEvent::RoleSpoke { role, .. }
        | CrepEvent::PhaseBlocked { role, .. }
        | CrepEvent::PlanReviewed { role, .. }
        | CrepEvent::PlanOverridden { role, .. }
        | CrepEvent::RoleOutputDelta { role, .. }
        | CrepEvent::TurnInterrupted { role, .. }
        | CrepEvent::ToolCallProposed { role, .. }
        | CrepEvent::ToolCallExecuted { role, .. }
        | CrepEvent::PermissionDenied { role, .. }
        | CrepEvent::RoleStopped { role, .. } => Some(role),
        CrepEvent::PhaseAdvanced { .. } => None,
    }
}

fn crep_event_turn(event: &CrepEvent) -> Option<&str> {
    match event {
        CrepEvent::TurnDispatched { turn_id, .. }
        | CrepEvent::WorkTitle { turn_id, .. }
        | CrepEvent::RoleSpoke { turn_id, .. }
        | CrepEvent::RoleOutputDelta { turn_id, .. }
        | CrepEvent::TurnInterrupted { turn_id, .. }
        | CrepEvent::ToolCallProposed { turn_id, .. }
        | CrepEvent::ToolCallExecuted { turn_id, .. }
        | CrepEvent::PermissionDenied { turn_id, .. } => nonempty_str(turn_id),
        CrepEvent::RoleStopped { turn_id, .. } => turn_id.as_deref().and_then(nonempty_str),
        CrepEvent::RoleStarted { .. }
        | CrepEvent::RoleSessionUpdated { .. }
        | CrepEvent::PhaseAdvanced { .. }
        | CrepEvent::PhaseBlocked { .. }
        | CrepEvent::PlanReviewed { .. }
        | CrepEvent::PlanOverridden { .. } => None,
    }
}

fn crep_event_thread(event: &CrepEvent) -> Option<&str> {
    match event {
        CrepEvent::TurnDispatched { thread_id, .. }
        | CrepEvent::WorkTitle { thread_id, .. }
        | CrepEvent::RoleSpoke { thread_id, .. }
        | CrepEvent::RoleOutputDelta { thread_id, .. }
        | CrepEvent::TurnInterrupted { thread_id, .. }
        | CrepEvent::ToolCallProposed { thread_id, .. }
        | CrepEvent::ToolCallExecuted { thread_id, .. }
        | CrepEvent::PermissionDenied { thread_id, .. } => nonempty_str(thread_id),
        CrepEvent::RoleStarted { .. }
        | CrepEvent::RoleSessionUpdated { .. }
        | CrepEvent::PhaseAdvanced { .. }
        | CrepEvent::PhaseBlocked { .. }
        | CrepEvent::PlanReviewed { .. }
        | CrepEvent::PlanOverridden { .. }
        | CrepEvent::RoleStopped { .. } => None,
    }
}

fn crep_event_summary(event: &CrepEvent) -> String {
    match event {
        CrepEvent::RoleStarted {
            engine,
            model,
            session_id,
            ..
        } => format!("started {engine}/{model} session {session_id}"),
        CrepEvent::RoleSessionUpdated { session_id, .. } => {
            format!("session updated to {session_id}")
        }
        CrepEvent::TurnDispatched { queue_position, .. } => {
            format!("turn dispatched at queue position {queue_position}")
        }
        CrepEvent::WorkTitle { title, .. } => title.clone(),
        CrepEvent::RoleSpoke {
            text,
            mentions,
            outcome,
            ..
        } => format!(
            "{}; mentions: {}; outcome: {}",
            compact_text(text),
            mentions.join(", "),
            outcome.label()
        ),
        CrepEvent::PhaseAdvanced {
            from, to, actor, ..
        } => {
            format!("phase advanced {from:?} -> {to:?} by {actor}")
        }
        CrepEvent::PhaseBlocked { phase, reason, .. } => {
            format!("{phase:?} blocked: {reason}")
        }
        CrepEvent::PlanReviewed {
            decision, plan_sha, ..
        } => format!("plan {plan_sha} reviewed as {decision:?}"),
        CrepEvent::PlanOverridden { reason, .. } => format!("plan override: {reason}"),
        CrepEvent::RoleOutputDelta {
            sequence,
            text_delta,
            ..
        } => {
            format!("delta #{sequence}: {}", compact_text(text_delta))
        }
        CrepEvent::TurnInterrupted { source, .. } => format!("turn interrupted by {source:?}"),
        CrepEvent::ToolCallProposed {
            tool_name,
            tool_use_id,
            ..
        } => {
            format!("proposed {tool_name} ({tool_use_id})")
        }
        CrepEvent::ToolCallExecuted {
            tool_use_id,
            ok,
            output_summary,
            ..
        } => format!("{tool_use_id} ok={ok}: {output_summary}"),
        CrepEvent::PermissionDenied {
            tool_name, reason, ..
        } => {
            format!("denied {tool_name}: {reason}")
        }
        CrepEvent::RoleStopped { reason, .. } => format!("role stopped: {reason:?}"),
    }
}

fn crep_event_internal(event: &CrepEvent) -> bool {
    !matches!(event, CrepEvent::RoleSpoke { .. })
}

fn crep_event_status(event: &CrepEvent) -> StatusState {
    match event {
        CrepEvent::PhaseBlocked { .. } | CrepEvent::PermissionDenied { .. } => {
            StatusState::Blocking
        }
        CrepEvent::TurnInterrupted { .. } => StatusState::Warn,
        CrepEvent::ToolCallExecuted { ok, .. } if !ok => StatusState::Blocking,
        CrepEvent::RoleStopped {
            reason: crate::crep::StopReason::Crashed | crate::crep::StopReason::TimedOut,
            ..
        } => StatusState::Blocking,
        _ => StatusState::Ok,
    }
}

fn nonempty_str(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}

fn compact_text(text: &str) -> String {
    const LIMIT: usize = 96;
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.len() <= LIMIT {
        compact
    } else {
        format!("{}...", &compact[..LIMIT])
    }
}

fn xray_step(
    name: impl Into<String>,
    value: impl Into<String>,
    status: StatusState,
    freshness: impl Into<String>,
    citations: Vec<String>,
) -> XrayStep {
    XrayStep {
        name: name.into(),
        value: value.into(),
        status,
        freshness: freshness.into(),
        citations,
    }
}

fn related_source_status(sources: &[&SourceHealthSnapshot]) -> StatusState {
    if sources.iter().any(|source| {
        matches!(
            source.status,
            SourceHealthState::Missing
                | SourceHealthState::TrustChanged
                | SourceHealthState::VisibilityDenied
        )
    }) {
        StatusState::Blocking
    } else if sources
        .iter()
        .any(|source| source.status == SourceHealthState::Stale)
    {
        StatusState::Warn
    } else if sources.is_empty() {
        StatusState::Unknown
    } else {
        StatusState::Ok
    }
}

fn status_for_lifecycle(lifecycle: WorkLifecycle) -> StatusState {
    match lifecycle {
        WorkLifecycle::FailedCi | WorkLifecycle::Blocked | WorkLifecycle::MergedTrackerStale => {
            StatusState::Blocking
        }
        WorkLifecycle::InReview | WorkLifecycle::InProgress | WorkLifecycle::Ready => {
            StatusState::Warn
        }
        WorkLifecycle::Closed => StatusState::Ok,
        WorkLifecycle::NotStarted => StatusState::Unknown,
    }
}

fn lifecycle_freshness(lifecycle: WorkLifecycle) -> &'static str {
    match lifecycle {
        WorkLifecycle::MergedTrackerStale => "tracker-stale",
        WorkLifecycle::Closed => "closed",
        WorkLifecycle::FailedCi => "failed-ci",
        WorkLifecycle::Blocked => "blocked",
        WorkLifecycle::InReview => "in-review",
        WorkLifecycle::InProgress => "in-progress",
        WorkLifecycle::Ready => "ready",
        WorkLifecycle::NotStarted => "not-started",
    }
}

fn presence_status(present: bool) -> StatusState {
    if present {
        StatusState::Ok
    } else {
        StatusState::Warn
    }
}

fn presence_freshness(present: bool) -> &'static str {
    if present {
        "present"
    } else {
        "missing"
    }
}

fn status_label(status: StatusState) -> &'static str {
    match status {
        StatusState::Ok => "ok",
        StatusState::Warn => "warn",
        StatusState::Blocking => "blocking",
        StatusState::Unknown => "unknown",
    }
}

fn status_freshness(status: StatusState) -> &'static str {
    match status {
        StatusState::Ok => "fresh",
        StatusState::Warn => "attention",
        StatusState::Blocking => "blocking",
        StatusState::Unknown => "not-observed",
    }
}

fn evidence_status_label(status: EvidenceClosureState) -> &'static str {
    match status {
        EvidenceClosureState::Complete => "complete",
        EvidenceClosureState::Incomplete => "incomplete",
        EvidenceClosureState::Missing => "missing",
        EvidenceClosureState::Unverified => "unverified",
    }
}

fn source_count_label(count: usize) -> String {
    match count {
        0 => "none".to_owned(),
        1 => "1 source".to_owned(),
        count => format!("{count} sources"),
    }
}

fn xray_freshness(
    lifecycle: WorkLifecycle,
    tracker_state: StatusState,
    source_status: StatusState,
) -> &'static str {
    if lifecycle == WorkLifecycle::MergedTrackerStale || tracker_state == StatusState::Blocking {
        "tracker-stale"
    } else if source_status == StatusState::Blocking {
        "source-blocking"
    } else if source_status == StatusState::Warn {
        "source-stale"
    } else if lifecycle == WorkLifecycle::Closed {
        "complete"
    } else {
        "in-progress"
    }
}
