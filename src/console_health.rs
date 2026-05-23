//! Actionable console health selectors.
//!
//! Selectors classify snapshot rows. Overview health signals intentionally keep
//! only items that can change `@host` next action.

use serde::{Deserialize, Serialize};

use crate::console_snapshot::{
    CoreRoomSnapshot, EvidenceClosureState, HealthSeverity, HealthSignal, RoleLaneState,
    SourceHealthState, StatusState, WorkLifecycle,
};
use crate::observation::{
    FreshnessState, Observation, ObservationAuthority, ObservationCitation, ObservationFreshness,
};

/// Selector families supported by the console data plane.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum HealthSelector {
    /// Any non-closed work.
    #[serde(rename = "work:active")]
    WorkActive,
    /// Blocked work.
    #[serde(rename = "work:blocked")]
    WorkBlocked,
    /// Work with failed CI.
    #[serde(rename = "work:failed-ci")]
    WorkFailedCi,
    /// Work merged but tracker/evidence closure is stale.
    #[serde(rename = "work:stale-tracker")]
    WorkStaleTracker,
    /// Work in review.
    #[serde(rename = "work:in-review")]
    WorkInReview,
    /// Closed work.
    #[serde(rename = "work:closed")]
    WorkClosed,
    /// Enabled roles.
    #[serde(rename = "role:enabled")]
    RoleEnabled,
    /// Working roles.
    #[serde(rename = "role:working")]
    RoleWorking,
    /// Roles waiting approval.
    #[serde(rename = "role:waiting-approval")]
    RoleWaitingApproval,
    /// Resumed/stale sessions.
    #[serde(rename = "role:resumed")]
    RoleResumed,
    /// Roles with risky permission mode.
    #[serde(rename = "role:permission-risk")]
    RolePermissionRisk,
    /// Active gates.
    #[serde(rename = "gate:active")]
    GateActive,
    /// Blocked gates.
    #[serde(rename = "gate:blocked")]
    GateBlocked,
    /// Gates missing reviews.
    #[serde(rename = "gate:missing-review")]
    GateMissingReview,
    /// Gates with stale plan SHA.
    #[serde(rename = "gate:stale-plan")]
    GateStalePlan,
    /// Missing evidence.
    #[serde(rename = "evidence:missing")]
    EvidenceMissing,
    /// Incomplete evidence.
    #[serde(rename = "evidence:incomplete")]
    EvidenceIncomplete,
    /// Unverified evidence.
    #[serde(rename = "evidence:unverified")]
    EvidenceUnverified,
    /// Evidence/tracker closure is stale.
    #[serde(rename = "evidence:tracker-stale")]
    EvidenceTrackerStale,
    /// Stale source.
    #[serde(rename = "source:stale")]
    SourceStale,
    /// Missing source.
    #[serde(rename = "source:missing")]
    SourceMissing,
    /// Trust changed.
    #[serde(rename = "source:trust-changed")]
    SourceTrustChanged,
    /// Visibility denied.
    #[serde(rename = "source:visibility-denied")]
    SourceVisibilityDenied,
}

impl HealthSelector {
    /// Stable label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::WorkActive => "work:active",
            Self::WorkBlocked => "work:blocked",
            Self::WorkFailedCi => "work:failed-ci",
            Self::WorkStaleTracker => "work:stale-tracker",
            Self::WorkInReview => "work:in-review",
            Self::WorkClosed => "work:closed",
            Self::RoleEnabled => "role:enabled",
            Self::RoleWorking => "role:working",
            Self::RoleWaitingApproval => "role:waiting-approval",
            Self::RoleResumed => "role:resumed",
            Self::RolePermissionRisk => "role:permission-risk",
            Self::GateActive => "gate:active",
            Self::GateBlocked => "gate:blocked",
            Self::GateMissingReview => "gate:missing-review",
            Self::GateStalePlan => "gate:stale-plan",
            Self::EvidenceMissing => "evidence:missing",
            Self::EvidenceIncomplete => "evidence:incomplete",
            Self::EvidenceUnverified => "evidence:unverified",
            Self::EvidenceTrackerStale => "evidence:tracker-stale",
            Self::SourceStale => "source:stale",
            Self::SourceMissing => "source:missing",
            Self::SourceTrustChanged => "source:trust-changed",
            Self::SourceVisibilityDenied => "source:visibility-denied",
        }
    }
}

/// Selector result ids for deterministic tests and right-rail counts.
#[must_use]
pub fn select_ids(snapshot: &CoreRoomSnapshot, selector: HealthSelector) -> Vec<String> {
    match selector {
        HealthSelector::WorkActive => snapshot
            .work
            .iter()
            .filter(|work| work.lifecycle != WorkLifecycle::Closed)
            .map(|work| work.id.clone())
            .collect(),
        HealthSelector::WorkBlocked => work_by_lifecycle(snapshot, WorkLifecycle::Blocked),
        HealthSelector::WorkFailedCi => work_by_lifecycle(snapshot, WorkLifecycle::FailedCi),
        HealthSelector::WorkStaleTracker => snapshot
            .work
            .iter()
            .filter(|work| {
                work.lifecycle == WorkLifecycle::MergedTrackerStale
                    || work.tracker_state == StatusState::Blocking
            })
            .map(|work| work.id.clone())
            .collect(),
        HealthSelector::WorkInReview => work_by_lifecycle(snapshot, WorkLifecycle::InReview),
        HealthSelector::WorkClosed => work_by_lifecycle(snapshot, WorkLifecycle::Closed),
        HealthSelector::RoleEnabled => snapshot
            .runtime
            .roles
            .iter()
            .filter(|role| role.enabled)
            .map(|role| role.role.clone())
            .collect(),
        HealthSelector::RoleWorking => snapshot
            .runtime
            .roles
            .iter()
            .filter(|role| role.state == RoleLaneState::Working)
            .map(|role| role.role.clone())
            .collect(),
        HealthSelector::RoleWaitingApproval => snapshot
            .runtime
            .roles
            .iter()
            .filter(|role| role.waiting_approval || role.state == RoleLaneState::WaitingApproval)
            .map(|role| role.role.clone())
            .collect(),
        HealthSelector::RoleResumed => snapshot
            .runtime
            .roles
            .iter()
            .filter(|role| {
                matches!(
                    role.state,
                    RoleLaneState::StaleSession | RoleLaneState::WaitingUser
                )
            })
            .map(|role| role.role.clone())
            .collect(),
        HealthSelector::RolePermissionRisk => snapshot
            .runtime
            .roles
            .iter()
            .filter(|role| role.permission_mode.as_deref() == Some("bypass"))
            .map(|role| role.role.clone())
            .collect(),
        HealthSelector::GateActive => snapshot
            .gates
            .iter()
            .map(|gate| gate.work_order.clone())
            .collect(),
        HealthSelector::GateBlocked => snapshot
            .gates
            .iter()
            .filter(|gate| gate.blocked_reason.is_some())
            .map(|gate| gate.work_order.clone())
            .collect(),
        HealthSelector::GateMissingReview => snapshot
            .gates
            .iter()
            .filter(|gate| !gate.missing_reviews.is_empty())
            .map(|gate| gate.work_order.clone())
            .collect(),
        HealthSelector::GateStalePlan => snapshot
            .gates
            .iter()
            .filter(|gate| gate.stale_plan_sha.is_some())
            .map(|gate| gate.work_order.clone())
            .collect(),
        HealthSelector::EvidenceMissing => {
            evidence_by_status(snapshot, EvidenceClosureState::Missing)
        }
        HealthSelector::EvidenceIncomplete => {
            evidence_by_status(snapshot, EvidenceClosureState::Incomplete)
        }
        HealthSelector::EvidenceUnverified => {
            evidence_by_status(snapshot, EvidenceClosureState::Unverified)
        }
        HealthSelector::EvidenceTrackerStale => snapshot
            .evidence
            .iter()
            .filter(|evidence| !evidence.tracker_updated)
            .map(|evidence| evidence.work_order.clone())
            .collect(),
        HealthSelector::SourceStale => source_by_status(snapshot, SourceHealthState::Stale),
        HealthSelector::SourceMissing => source_by_status(snapshot, SourceHealthState::Missing),
        HealthSelector::SourceTrustChanged => {
            source_by_status(snapshot, SourceHealthState::TrustChanged)
        }
        HealthSelector::SourceVisibilityDenied => {
            source_by_status(snapshot, SourceHealthState::VisibilityDenied)
        }
    }
}

/// Build action-relevant overview health signals.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn overview_health_signals(snapshot: &CoreRoomSnapshot) -> Vec<HealthSignal> {
    let mut signals = Vec::new();
    for work in &snapshot.work {
        match work.lifecycle {
            WorkLifecycle::Blocked => signals.push(signal(
                format!("work:blocked:{}", work.id),
                HealthSeverity::Blocking,
                format!("{} is blocked", work.id),
                format!("work:{}", work.id),
                "Ask the user or responsible role to resolve the blocker.",
            )),
            WorkLifecycle::FailedCi => signals.push(signal(
                format!("work:failed-ci:{}", work.id),
                HealthSeverity::Blocking,
                format!("{} has failed CI", work.id),
                format!("work:{}", work.id),
                "Inspect failing checks before continuing.",
            )),
            WorkLifecycle::MergedTrackerStale => signals.push(signal(
                format!("work:stale-tracker:{}", work.id),
                HealthSeverity::Blocking,
                format!("{} is merged but tracker/evidence is stale", work.id),
                format!("work:{}", work.id),
                "Update tracker checkbox and Evidence Ledger before claiming done.",
            )),
            WorkLifecycle::InReview
                if work.evidence_state != StatusState::Ok
                    || work.tracker_state != StatusState::Ok =>
            {
                signals.push(signal(
                    format!("work:in-review:{}", work.id),
                    HealthSeverity::Warn,
                    format!("{} is in review with incomplete closure evidence", work.id),
                    format!("work:{}", work.id),
                    "Keep PR/evidence/tracker state visible until merge.",
                ));
            }
            _ => {}
        }
    }
    for role in &snapshot.runtime.roles {
        if role.waiting_approval || role.state == RoleLaneState::WaitingApproval {
            signals.push(signal(
                format!("role:waiting-approval:{}", role.role),
                HealthSeverity::Warn,
                format!("@{} is waiting for approval", role.role),
                format!("role:{}", role.role),
                "Ask the user to approve, deny, or change the plan.",
            ));
        }
        if role.permission_mode.as_deref() == Some("bypass") {
            signals.push(signal(
                format!("role:permission-risk:{}", role.role),
                HealthSeverity::Warn,
                format!("@{} is running with bypass permissions", role.role),
                format!("role:{}", role.role),
                "Confirm bypass is intentional before sensitive work.",
            ));
        }
        if role.state == RoleLaneState::StaleSession {
            signals.push(signal(
                format!("role:resumed:{}", role.role),
                HealthSeverity::Warn,
                format!("@{} has a stale session", role.role),
                format!("role:{}", role.role),
                "Refresh role session before relying on its output.",
            ));
        }
    }
    for gate in &snapshot.gates {
        if let Some(reason) = &gate.blocked_reason {
            signals.push(signal(
                format!("gate:blocked:{}", gate.work_order),
                HealthSeverity::Blocking,
                format!("{} gate is blocked: {reason}", gate.work_order),
                format!("gate:{}", gate.work_order),
                "Resolve the gate blocker before implementation or closure.",
            ));
        }
        if !gate.missing_reviews.is_empty() {
            signals.push(signal(
                format!("gate:missing-review:{}", gate.work_order),
                HealthSeverity::Warn,
                format!("{} is missing required reviews", gate.work_order),
                format!("gate:{}", gate.work_order),
                "Request the missing authority-scoped reviews.",
            ));
        }
        if gate.stale_plan_sha.is_some() {
            signals.push(signal(
                format!("gate:stale-plan:{}", gate.work_order),
                HealthSeverity::Blocking,
                format!("{} has a stale plan review", gate.work_order),
                format!("gate:{}", gate.work_order),
                "Re-run plan review against the current plan SHA.",
            ));
        }
    }
    for evidence in &snapshot.evidence {
        match evidence.status {
            EvidenceClosureState::Missing => signals.push(signal(
                format!("evidence:missing:{}", evidence.work_order),
                HealthSeverity::Blocking,
                format!("{} is missing evidence", evidence.work_order),
                format!("evidence:{}", evidence.work_order),
                "Collect an Evidence Packet before closure.",
            )),
            EvidenceClosureState::Incomplete => signals.push(signal(
                format!("evidence:incomplete:{}", evidence.work_order),
                HealthSeverity::Warn,
                format!("{} evidence is incomplete", evidence.work_order),
                format!("evidence:{}", evidence.work_order),
                "Fill missing evidence fields before claiming done.",
            )),
            EvidenceClosureState::Unverified => signals.push(signal(
                format!("evidence:unverified:{}", evidence.work_order),
                HealthSeverity::Warn,
                format!("{} has unverified evidence", evidence.work_order),
                format!("evidence:{}", evidence.work_order),
                "State what remains unverified and why.",
            )),
            EvidenceClosureState::Complete => {}
        }
        if !evidence.tracker_updated {
            signals.push(signal(
                format!("evidence:tracker-stale:{}", evidence.work_order),
                HealthSeverity::Blocking,
                format!("{} tracker row is stale", evidence.work_order),
                format!("evidence:{}", evidence.work_order),
                "Update tracker row and Evidence Ledger.",
            ));
        }
    }
    for source in &snapshot.sources {
        match source.status {
            SourceHealthState::Stale => signals.push(signal(
                format!("source:stale:{}", source.source_id),
                HealthSeverity::Warn,
                format!("{} source is stale", source.source_id),
                format!("source:{}", source.source_id),
                "Ask before refreshing the pinned source.",
            )),
            SourceHealthState::Missing => signals.push(signal(
                format!("source:missing:{}", source.source_id),
                HealthSeverity::Blocking,
                format!("{} source is missing", source.source_id),
                format!("source:{}", source.source_id),
                "Restore or remove the missing source before using it.",
            )),
            SourceHealthState::TrustChanged => signals.push(signal(
                format!("source:trust-changed:{}", source.source_id),
                HealthSeverity::Blocking,
                format!("{} source trust changed", source.source_id),
                format!("source:{}", source.source_id),
                "Reconfirm trust level before delegating with this source.",
            )),
            SourceHealthState::VisibilityDenied => signals.push(signal(
                format!("source:visibility-denied:{}", source.source_id),
                HealthSeverity::Blocking,
                format!("{} source visibility is denied", source.source_id),
                format!("source:{}", source.source_id),
                "Fix source visibility before building a ContextPack.",
            )),
            SourceHealthState::Pinned => {}
        }
    }
    if snapshot.github.failing_checks > 0 {
        signals.push(signal(
            "github:failing-checks".to_owned(),
            HealthSeverity::Blocking,
            format!(
                "{} GitHub check(s) are failing",
                snapshot.github.failing_checks
            ),
            "github".to_owned(),
            "Inspect failing GitHub checks before merge or release.",
        ));
    }
    signals
}

fn work_by_lifecycle(snapshot: &CoreRoomSnapshot, lifecycle: WorkLifecycle) -> Vec<String> {
    snapshot
        .work
        .iter()
        .filter(|work| work.lifecycle == lifecycle)
        .map(|work| work.id.clone())
        .collect()
}

fn evidence_by_status(snapshot: &CoreRoomSnapshot, status: EvidenceClosureState) -> Vec<String> {
    snapshot
        .evidence
        .iter()
        .filter(|evidence| evidence.status == status)
        .map(|evidence| evidence.work_order.clone())
        .collect()
}

fn source_by_status(snapshot: &CoreRoomSnapshot, status: SourceHealthState) -> Vec<String> {
    snapshot
        .sources
        .iter()
        .filter(|source| source.status == status)
        .map(|source| source.source_id.clone())
        .collect()
}

fn signal(
    id: impl Into<String>,
    severity: HealthSeverity,
    title: impl Into<String>,
    source: impl Into<String>,
    next_action: &str,
) -> HealthSignal {
    let id = id.into();
    let title = title.into();
    let source = source.into();
    HealthSignal {
        id: id.clone(),
        severity,
        title: title.clone(),
        source: source.clone(),
        next_action: Some(next_action.to_owned()),
        observations: vec![Observation {
            id: format!("obs-{id}"),
            summary: title,
            authority: ObservationAuthority::Generated,
            freshness: ObservationFreshness {
                state: FreshnessState::Unknown,
                observed_at: None,
                max_age_seconds: None,
                missing_reason: Some(
                    "Derived from the in-memory CoreRoomSnapshot; live freshness is provided by upstream facts."
                        .to_owned(),
                ),
            },
            citations: vec![ObservationCitation::Generated {
                artifact: "CoreRoomSnapshot".to_owned(),
                source,
                observed_at: None,
                missing_freshness: Some("Snapshot selector has no wall-clock observation.".to_owned()),
            }],
        }],
    }
}
