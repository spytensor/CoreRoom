//! K9s-style overview model for the CoreRoom console.
//!
//! The overview is a pure projection over [`CoreRoomSnapshot`]. It does not
//! invent live facts, poll external systems, or scrape chat prose.

use crate::console_health::overview_health_signals;
use crate::console_snapshot::{
    CoreRoomSnapshot, DirtyState, EvidenceClosureState, GateSnapshot, HealthSeverity,
    RoleLaneState, SourceHealthState, StatusState, WorkLifecycle,
};

/// Project and runtime facts shown in the overview header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverviewHeader {
    /// Product or project name.
    pub project: String,
    /// Repository in owner/name form.
    pub repository: String,
    /// Current branch.
    pub branch: String,
    /// Active phase.
    pub phase: String,
    /// Configured host role.
    pub host_role: String,
    /// Active tracker issue.
    pub tracker_issue: u64,
    /// Worktree dirty state.
    pub dirty_state: DirtyState,
    /// Open issues from the snapshot.
    pub open_issues: u32,
    /// Open PRs from the snapshot.
    pub open_pull_requests: u32,
    /// Failing checks from the snapshot.
    pub failing_checks: u32,
}

/// One actionable pulse in the room overview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverviewPulse {
    /// Stable pulse id.
    pub id: &'static str,
    /// Display label.
    pub label: &'static str,
    /// Source path inside `CoreRoomSnapshot`.
    pub source: &'static str,
    /// Number of rows represented by this pulse.
    pub total: usize,
    /// Healthy rows.
    pub ok: usize,
    /// Warning rows.
    pub warn: usize,
    /// Blocking rows.
    pub blocking: usize,
    /// Unknown rows.
    pub unknown: usize,
    /// Recommended next action when attention is needed.
    pub next_action: Option<String>,
}

/// One action-relevant overview alert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverviewAlert {
    /// Stable alert id.
    pub id: String,
    /// Alert severity.
    pub severity: HealthSeverity,
    /// Alert title.
    pub title: String,
    /// Structural source label.
    pub source: String,
    /// Recommended next action.
    pub next_action: Option<String>,
}

/// Complete room overview for the console first screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleOverview {
    /// Header facts.
    pub header: OverviewHeader,
    /// Overview pulses in stable display order.
    pub pulses: Vec<OverviewPulse>,
    /// Action-relevant alerts in severity order.
    pub alerts: Vec<OverviewAlert>,
}

/// Build the console room overview from a validated snapshot.
#[must_use]
pub fn build_console_overview(snapshot: &CoreRoomSnapshot) -> ConsoleOverview {
    let header = OverviewHeader {
        project: snapshot.project.project.clone(),
        repository: snapshot.project.repository.clone(),
        branch: snapshot.project.branch.clone(),
        phase: snapshot.project.active_phase.clone(),
        host_role: snapshot.runtime.host_role.clone(),
        tracker_issue: snapshot.project.tracker_issue,
        dirty_state: snapshot.project.dirty_state,
        open_issues: snapshot.github.open_issues,
        open_pull_requests: snapshot.github.open_pull_requests,
        failing_checks: snapshot.github.failing_checks,
    };
    let mut alerts = overview_health_signals(snapshot)
        .into_iter()
        .chain(snapshot.alerts.clone())
        .filter(|signal| {
            signal.severity != HealthSeverity::Ok
                || signal
                    .next_action
                    .as_deref()
                    .is_some_and(|action| !action.is_empty())
        })
        .map(|signal| OverviewAlert {
            id: signal.id,
            severity: signal.severity,
            title: signal.title,
            source: signal.source,
            next_action: signal.next_action,
        })
        .collect::<Vec<_>>();
    alerts.sort_by_key(|alert| severity_rank(alert.severity));

    ConsoleOverview {
        header,
        pulses: vec![
            roles_pulse(snapshot),
            work_pulse(snapshot),
            gates_pulse(snapshot),
            evidence_pulse(snapshot),
            sources_pulse(snapshot),
        ],
        alerts,
    }
}

fn roles_pulse(snapshot: &CoreRoomSnapshot) -> OverviewPulse {
    let mut pulse = OverviewPulse::new("roles", "Roles", "snapshot.runtime.roles");
    for role in &snapshot.runtime.roles {
        match role.state {
            RoleLaneState::Blocked => pulse.blocking += 1,
            RoleLaneState::WaitingApproval
            | RoleLaneState::WaitingUser
            | RoleLaneState::StaleSession => pulse.warn += 1,
            RoleLaneState::Enabled
            | RoleLaneState::Idle
            | RoleLaneState::Working
            | RoleLaneState::Reviewing => {
                if role.waiting_approval {
                    pulse.warn += 1;
                } else {
                    pulse.ok += 1;
                }
            }
        }
    }
    pulse.total = snapshot.runtime.roles.len();
    pulse.finalize_action();
    pulse
}

fn work_pulse(snapshot: &CoreRoomSnapshot) -> OverviewPulse {
    let mut pulse = OverviewPulse::new("work", "WorkOrders", "snapshot.work");
    for work in &snapshot.work {
        match work.lifecycle {
            WorkLifecycle::Closed => pulse.ok += 1,
            WorkLifecycle::Blocked
            | WorkLifecycle::FailedCi
            | WorkLifecycle::MergedTrackerStale => pulse.blocking += 1,
            WorkLifecycle::Ready | WorkLifecycle::InProgress | WorkLifecycle::InReview => {
                pulse.warn += 1;
            }
            WorkLifecycle::NotStarted => pulse.unknown += 1,
        }
    }
    pulse.total = snapshot.work.len();
    pulse.finalize_action();
    pulse
}

fn gates_pulse(snapshot: &CoreRoomSnapshot) -> OverviewPulse {
    let mut pulse = OverviewPulse::new("gates", "Gates", "snapshot.gates");
    for gate in &snapshot.gates {
        match gate_state(gate) {
            StatusState::Ok => pulse.ok += 1,
            StatusState::Warn => pulse.warn += 1,
            StatusState::Blocking => pulse.blocking += 1,
            StatusState::Unknown => pulse.unknown += 1,
        }
    }
    pulse.total = snapshot.gates.len();
    pulse.finalize_action();
    pulse
}

fn evidence_pulse(snapshot: &CoreRoomSnapshot) -> OverviewPulse {
    let mut pulse = OverviewPulse::new("evidence", "Evidence", "snapshot.evidence");
    for evidence in &snapshot.evidence {
        match evidence.status {
            EvidenceClosureState::Complete if evidence.tracker_updated => pulse.ok += 1,
            EvidenceClosureState::Missing => pulse.blocking += 1,
            EvidenceClosureState::Complete
            | EvidenceClosureState::Incomplete
            | EvidenceClosureState::Unverified => pulse.warn += 1,
        }
    }
    pulse.total = snapshot.evidence.len();
    pulse.finalize_action();
    pulse
}

fn sources_pulse(snapshot: &CoreRoomSnapshot) -> OverviewPulse {
    let mut pulse = OverviewPulse::new("sources", "Sources", "snapshot.sources");
    for source in &snapshot.sources {
        match source.status {
            SourceHealthState::Pinned => pulse.ok += 1,
            SourceHealthState::Stale => pulse.warn += 1,
            SourceHealthState::Missing
            | SourceHealthState::TrustChanged
            | SourceHealthState::VisibilityDenied => pulse.blocking += 1,
        }
    }
    pulse.total = snapshot.sources.len();
    pulse.finalize_action();
    pulse
}

fn gate_state(gate: &GateSnapshot) -> StatusState {
    if gate.blocked_reason.is_some() || !gate.missing_reviews.is_empty() {
        StatusState::Blocking
    } else if gate.stale_plan_sha.is_some() || !gate.signoff_ready {
        StatusState::Warn
    } else {
        StatusState::Ok
    }
}

impl OverviewPulse {
    fn new(id: &'static str, label: &'static str, source: &'static str) -> Self {
        Self {
            id,
            label,
            source,
            total: 0,
            ok: 0,
            warn: 0,
            blocking: 0,
            unknown: 0,
            next_action: None,
        }
    }

    fn finalize_action(&mut self) {
        self.next_action = if self.blocking > 0 {
            Some(format!("inspect {} blockers", self.label))
        } else if self.warn > 0 {
            Some(format!("review {} warnings", self.label))
        } else if self.unknown > 0 {
            Some(format!("resolve {} unknowns", self.label))
        } else {
            None
        };
    }
}

fn severity_rank(severity: HealthSeverity) -> u8 {
    match severity {
        HealthSeverity::Blocking => 0,
        HealthSeverity::Warn => 1,
        HealthSeverity::Unknown => 2,
        HealthSeverity::Ok => 3,
    }
}
