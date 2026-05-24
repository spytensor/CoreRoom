//! Renderer-independent console layout and right-rail view models.
//!
//! v0.8 keeps this pure data-plane logic. The future v0.9 ratatui renderer can
//! consume these decisions without re-deciding which facts belong in the public
//! transcript versus side rails.

use serde::{Deserialize, Serialize};

use crate::console_health::overview_health_signals;
use crate::console_snapshot::{
    CoreRoomSnapshot, DirtyState, EvidenceClosureState, HealthSeverity, RoleLaneState,
    SourceHealthState, StatusState, WorkLifecycle,
};

/// Supported console breakpoint.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum ConsoleBreakpoint {
    /// Below the v0.8 minimum. Keep the conversation usable and hide side rails.
    SubMinimum,
    /// Around 120 columns.
    Compact120,
    /// Around 160 columns.
    Standard160,
    /// 220+ columns.
    Wide220,
}

/// Console pane id.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum ConsolePaneId {
    /// Project/repository header.
    ProjectHeader,
    /// User-facing transcript.
    PublicConversation,
    /// Right status rail.
    RightRail,
    /// Project facts panel.
    ProjectState,
    /// Role lane summary panel.
    RoleLanes,
    /// Work list.
    WorkList,
    /// Gate pipeline.
    GatePipeline,
    /// Evidence closure panel.
    EvidenceClosure,
    /// Source health panel.
    Sources,
    /// Alerts/health panel.
    Alerts,
    /// Footer/tab bar.
    Tabs,
    /// Debug or Xray detail pane.
    DebugLog,
}

impl ConsolePaneId {
    /// Stable pane label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::ProjectHeader => "project-header",
            Self::PublicConversation => "public-conversation",
            Self::RightRail => "right-rail",
            Self::ProjectState => "project-state",
            Self::RoleLanes => "role-lanes",
            Self::WorkList => "work-list",
            Self::GatePipeline => "gate-pipeline",
            Self::EvidenceClosure => "evidence-closure",
            Self::Sources => "sources",
            Self::Alerts => "alerts",
            Self::Tabs => "tabs",
            Self::DebugLog => "debug-log",
        }
    }
}

/// Pane placement decision.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum PanePlacement {
    /// Top header region.
    Header,
    /// Main center region.
    Center,
    /// Right rail region.
    Right,
    /// Left rail region.
    Left,
    /// Bottom tab/footer region.
    Footer,
    /// Facts are folded into the right rail as sections.
    FoldedIntoRightRail,
    /// Pane is hidden at this width.
    Hidden,
}

/// Static pane priority.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PanePriority {
    /// Pane id.
    pub pane: ConsolePaneId,
    /// Lower numbers are more important.
    pub priority: u8,
    /// Minimum columns before this pane can become a standalone pane.
    pub standalone_min_columns: u16,
    /// Why this pane exists.
    pub purpose: String,
}

/// One computed pane decision.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PaneDecision {
    /// Pane id.
    pub pane: ConsolePaneId,
    /// Placement at the requested width.
    pub placement: PanePlacement,
    /// Whether the pane is rendered as its own pane.
    pub visible: bool,
    /// Lower numbers are more important.
    pub priority: u8,
    /// Computed column width for standalone panes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub columns: Option<u16>,
    /// Short reason for the decision.
    pub reason: String,
}

/// Right rail section kind.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum RightRailSectionKind {
    /// Active WorkOrder and progress summary.
    ProgressWork,
    /// Repository, branch, tracker, and phase.
    Environment,
    /// Dirty state, PRs, issues, and failing checks.
    Changes,
    /// Active/attention-needing role lanes.
    ActiveRoles,
    /// Evidence closure status.
    Evidence,
    /// Source health status.
    Sources,
    /// Actionable alerts.
    Alerts,
}

impl RightRailSectionKind {
    /// Stable label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::ProgressWork => "progress-work",
            Self::Environment => "environment",
            Self::Changes => "changes",
            Self::ActiveRoles => "active-roles",
            Self::Evidence => "evidence",
            Self::Sources => "sources",
            Self::Alerts => "alerts",
        }
    }
}

/// One row in a right rail section.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RightRailRow {
    /// Compact label.
    pub label: String,
    /// Compact value.
    pub value: String,
    /// Optional status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<StatusState>,
    /// Optional next action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// Optional source/citation label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// One right rail section.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RightRailSection {
    /// Section kind.
    pub kind: RightRailSectionKind,
    /// Section title.
    pub title: String,
    /// Section rows.
    pub rows: Vec<RightRailRow>,
}

/// Right rail view model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RightRailViewModel {
    /// Width assigned to the right rail.
    pub columns: u16,
    /// Sections in render order.
    pub sections: Vec<RightRailSection>,
}

/// Computed console layout.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleLayoutModel {
    /// Input terminal columns.
    pub terminal_columns: u16,
    /// Breakpoint decision.
    pub breakpoint: ConsoleBreakpoint,
    /// Conversation panel width. This is kept usable before secondary panels.
    pub conversation_columns: u16,
    /// Pane decisions.
    pub panes: Vec<PaneDecision>,
    /// Right rail if visible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right_rail: Option<RightRailViewModel>,
}

/// Static pane priorities for documentation and tests.
#[must_use]
pub fn pane_priorities() -> Vec<PanePriority> {
    vec![
        priority(
            ConsolePaneId::PublicConversation,
            0,
            0,
            "Primary @user <-> @host transcript.",
        ),
        priority(
            ConsolePaneId::ProjectHeader,
            1,
            0,
            "Project, repository, branch, version, and tracker identity.",
        ),
        priority(ConsolePaneId::Tabs, 2, 0, "Navigation context."),
        priority(
            ConsolePaneId::RightRail,
            3,
            120,
            "Actionable status summary that does not pollute the transcript.",
        ),
        priority(
            ConsolePaneId::Alerts,
            4,
            120,
            "Only health signals that change @host next action.",
        ),
        priority(
            ConsolePaneId::WorkList,
            5,
            160,
            "Active WorkOrder progress and closure state.",
        ),
        priority(
            ConsolePaneId::RoleLanes,
            6,
            220,
            "Enabled roles and current activity without public chat noise.",
        ),
        priority(
            ConsolePaneId::ProjectState,
            7,
            220,
            "Expanded project/repo/milestone facts.",
        ),
        priority(
            ConsolePaneId::GatePipeline,
            8,
            160,
            "Gate blockers and missing authority reviews.",
        ),
        priority(
            ConsolePaneId::EvidenceClosure,
            9,
            160,
            "Evidence Packet and tracker closure state.",
        ),
        priority(
            ConsolePaneId::Sources,
            10,
            160,
            "Registered dependency/source health.",
        ),
        priority(
            ConsolePaneId::DebugLog,
            99,
            220,
            "Xray/debug detail opened on demand only.",
        ),
    ]
}

/// Compute layout and right-rail state for a terminal width.
#[must_use]
pub fn compute_console_layout(
    snapshot: &CoreRoomSnapshot,
    terminal_columns: u16,
) -> ConsoleLayoutModel {
    let breakpoint = breakpoint_for(terminal_columns);
    let right_columns = right_rail_columns(breakpoint);
    let left_columns = left_rail_columns(breakpoint);
    let gutters = if right_columns > 0 && left_columns > 0 {
        4
    } else if right_columns > 0 || left_columns > 0 {
        2
    } else {
        0
    };
    let conversation_columns = terminal_columns
        .saturating_sub(right_columns)
        .saturating_sub(left_columns)
        .saturating_sub(gutters)
        .max(match breakpoint {
            ConsoleBreakpoint::SubMinimum => terminal_columns,
            ConsoleBreakpoint::Compact120 => 76,
            ConsoleBreakpoint::Standard160 => 104,
            ConsoleBreakpoint::Wide220 => 132,
        });
    let right_rail = (right_columns > 0).then(|| RightRailViewModel {
        columns: right_columns,
        sections: right_rail_sections(snapshot, breakpoint),
    });
    ConsoleLayoutModel {
        terminal_columns,
        breakpoint,
        conversation_columns,
        panes: pane_decisions(
            breakpoint,
            conversation_columns,
            right_columns,
            left_columns,
        ),
        right_rail,
    }
}

fn priority(
    pane: ConsolePaneId,
    priority: u8,
    standalone_min_columns: u16,
    purpose: &str,
) -> PanePriority {
    PanePriority {
        pane,
        priority,
        standalone_min_columns,
        purpose: purpose.to_owned(),
    }
}

fn breakpoint_for(columns: u16) -> ConsoleBreakpoint {
    match columns {
        0..=119 => ConsoleBreakpoint::SubMinimum,
        120..=159 => ConsoleBreakpoint::Compact120,
        160..=219 => ConsoleBreakpoint::Standard160,
        _ => ConsoleBreakpoint::Wide220,
    }
}

fn right_rail_columns(breakpoint: ConsoleBreakpoint) -> u16 {
    match breakpoint {
        ConsoleBreakpoint::SubMinimum => 0,
        ConsoleBreakpoint::Compact120 => 36,
        ConsoleBreakpoint::Standard160 => 44,
        ConsoleBreakpoint::Wide220 => 48,
    }
}

fn left_rail_columns(breakpoint: ConsoleBreakpoint) -> u16 {
    match breakpoint {
        ConsoleBreakpoint::Wide220 => 34,
        ConsoleBreakpoint::SubMinimum
        | ConsoleBreakpoint::Compact120
        | ConsoleBreakpoint::Standard160 => 0,
    }
}

fn pane_decisions(
    breakpoint: ConsoleBreakpoint,
    conversation_columns: u16,
    right_columns: u16,
    left_columns: u16,
) -> Vec<PaneDecision> {
    pane_priorities()
        .into_iter()
        .map(|pane| {
            let (placement, visible, columns, reason) = decision_for(
                pane.pane,
                breakpoint,
                conversation_columns,
                right_columns,
                left_columns,
            );
            PaneDecision {
                pane: pane.pane,
                placement,
                visible,
                priority: pane.priority,
                columns,
                reason,
            }
        })
        .collect()
}

fn decision_for(
    pane: ConsolePaneId,
    breakpoint: ConsoleBreakpoint,
    conversation_columns: u16,
    right_columns: u16,
    left_columns: u16,
) -> (PanePlacement, bool, Option<u16>, String) {
    match pane {
        ConsolePaneId::ProjectHeader => (
            PanePlacement::Header,
            true,
            None,
            "Always visible to anchor project/repo/branch context.".to_owned(),
        ),
        ConsolePaneId::PublicConversation => (
            PanePlacement::Center,
            true,
            Some(conversation_columns),
            "Highest-priority center panel for @user <-> @host.".to_owned(),
        ),
        ConsolePaneId::Tabs => (
            PanePlacement::Footer,
            true,
            None,
            "Tabs remain visible as navigation context.".to_owned(),
        ),
        ConsolePaneId::RightRail if right_columns > 0 => (
            PanePlacement::Right,
            true,
            Some(right_columns),
            "Visible once width can preserve a usable conversation.".to_owned(),
        ),
        ConsolePaneId::RightRail => (
            PanePlacement::Hidden,
            false,
            None,
            "Hidden below 120 columns to protect conversation width.".to_owned(),
        ),
        ConsolePaneId::ProjectState | ConsolePaneId::RoleLanes
            if breakpoint == ConsoleBreakpoint::Wide220 =>
        {
            (
                PanePlacement::Left,
                true,
                Some(left_columns),
                "Wide layout can afford a left project/role rail.".to_owned(),
            )
        }
        ConsolePaneId::ProjectState | ConsolePaneId::RoleLanes => (
            PanePlacement::FoldedIntoRightRail,
            false,
            None,
            "Folded into right rail summary before 220 columns.".to_owned(),
        ),
        ConsolePaneId::WorkList
        | ConsolePaneId::GatePipeline
        | ConsolePaneId::EvidenceClosure
        | ConsolePaneId::Sources
        | ConsolePaneId::Alerts
            if right_columns > 0 =>
        {
            (
                PanePlacement::FoldedIntoRightRail,
                false,
                None,
                "Rendered as compact right-rail sections to keep transcript clean.".to_owned(),
            )
        }
        ConsolePaneId::WorkList
        | ConsolePaneId::GatePipeline
        | ConsolePaneId::EvidenceClosure
        | ConsolePaneId::Sources
        | ConsolePaneId::Alerts => (
            PanePlacement::Hidden,
            false,
            None,
            "Hidden below 120 columns instead of squeezing public conversation.".to_owned(),
        ),
        ConsolePaneId::DebugLog => (
            PanePlacement::Hidden,
            false,
            None,
            "Debug/Xray is opened on demand, never default noise.".to_owned(),
        ),
    }
}

fn right_rail_sections(
    snapshot: &CoreRoomSnapshot,
    breakpoint: ConsoleBreakpoint,
) -> Vec<RightRailSection> {
    let mut sections = Vec::new();
    push_if_nonempty(&mut sections, progress_section(snapshot, breakpoint));
    push_if_nonempty(&mut sections, environment_section(snapshot));
    push_if_nonempty(&mut sections, changes_section(snapshot));
    push_if_nonempty(&mut sections, active_roles_section(snapshot, breakpoint));
    push_if_nonempty(&mut sections, evidence_section(snapshot, breakpoint));
    push_if_nonempty(&mut sections, sources_section(snapshot, breakpoint));
    push_if_nonempty(&mut sections, alerts_section(snapshot, breakpoint));
    sections
}

fn push_if_nonempty(sections: &mut Vec<RightRailSection>, section: RightRailSection) {
    if !section.rows.is_empty() {
        sections.push(section);
    }
}

fn progress_section(
    snapshot: &CoreRoomSnapshot,
    breakpoint: ConsoleBreakpoint,
) -> RightRailSection {
    let active = snapshot
        .work
        .iter()
        .filter(|work| work.lifecycle != WorkLifecycle::Closed)
        .count();
    let blocked = snapshot
        .work
        .iter()
        .filter(|work| work.lifecycle == WorkLifecycle::Blocked)
        .count();
    let in_review = snapshot
        .work
        .iter()
        .filter(|work| work.lifecycle == WorkLifecycle::InReview)
        .count();
    let closed = snapshot
        .work
        .iter()
        .filter(|work| work.lifecycle == WorkLifecycle::Closed)
        .count();
    let mut rows = vec![
        row(
            "active",
            active.to_string(),
            state_from_count(blocked),
            None,
            None,
        ),
        row("in review", in_review.to_string(), None, None, None),
        row(
            "closed",
            closed.to_string(),
            Some(StatusState::Ok),
            None,
            None,
        ),
    ];
    rows.extend(
        snapshot
            .work
            .iter()
            .filter(|work| work.lifecycle != WorkLifecycle::Closed)
            .take(row_limit(breakpoint))
            .map(|work| {
                row(
                    work.id.clone(),
                    work.title.clone(),
                    Some(status_for_lifecycle(work.lifecycle)),
                    Some("advance or clear this WorkOrder before claiming progress".to_owned()),
                    work.github_issue.map(|issue| format!("issue:#{issue}")),
                )
            }),
    );
    RightRailSection {
        kind: RightRailSectionKind::ProgressWork,
        title: "Progress".to_owned(),
        rows,
    }
}

fn environment_section(snapshot: &CoreRoomSnapshot) -> RightRailSection {
    let project = &snapshot.project;
    RightRailSection {
        kind: RightRailSectionKind::Environment,
        title: "Environment".to_owned(),
        rows: vec![
            row(
                "repo",
                project.repository.clone(),
                None,
                None,
                project.remote.clone(),
            ),
            row(
                "branch",
                project.branch.clone(),
                None,
                None,
                project.head_sha.clone(),
            ),
            row(
                "phase",
                project.active_phase.clone(),
                None,
                None,
                Some(format!("tracker:#{}", project.tracker_issue)),
            ),
            row(
                "version",
                project.version.clone(),
                None,
                None,
                Some(dirty_state_label(project.dirty_state).to_owned()),
            ),
        ],
    }
}

fn changes_section(snapshot: &CoreRoomSnapshot) -> RightRailSection {
    let github = &snapshot.github;
    RightRailSection {
        kind: RightRailSectionKind::Changes,
        title: "Changes".to_owned(),
        rows: vec![
            row(
                "open PRs",
                github.open_pull_requests.to_string(),
                state_from_count(github.failing_checks as usize),
                None,
                Some(format!("tracker:#{}", github.tracker_issue)),
            ),
            row(
                "open issues",
                github.open_issues.to_string(),
                None,
                None,
                None,
            ),
            row(
                "failing checks",
                github.failing_checks.to_string(),
                state_from_count(github.failing_checks as usize),
                (github.failing_checks > 0).then(|| "inspect checks before merge".to_owned()),
                None,
            ),
            row(
                "changed files",
                changed_files_label(snapshot.project.dirty_state),
                Some(dirty_state(snapshot.project.dirty_state)),
                None,
                Some(dirty_state_label(snapshot.project.dirty_state).to_owned()),
            ),
        ],
    }
}

fn active_roles_section(
    snapshot: &CoreRoomSnapshot,
    breakpoint: ConsoleBreakpoint,
) -> RightRailSection {
    let mut rows = vec![row(
        "enabled",
        snapshot
            .runtime
            .roles
            .iter()
            .filter(|role| role.enabled)
            .count()
            .to_string(),
        None,
        None,
        Some(format!("@{}", snapshot.runtime.host_role)),
    )];
    rows.extend(
        snapshot
            .runtime
            .roles
            .iter()
            .filter(|role| {
                role.role == snapshot.runtime.host_role
                    || !matches!(role.state, RoleLaneState::Enabled | RoleLaneState::Idle)
                    || role.waiting_approval
            })
            .take(row_limit(breakpoint))
            .map(|role| {
                row(
                    format!("@{}", role.role),
                    role_state_label(role.state),
                    Some(role_status(
                        role.state,
                        role.waiting_approval,
                        role.permission_mode.as_deref(),
                    )),
                    role.last_activity.clone(),
                    role.current_work_order.clone(),
                )
            }),
    );
    RightRailSection {
        kind: RightRailSectionKind::ActiveRoles,
        title: "Roles".to_owned(),
        rows,
    }
}

fn evidence_section(
    snapshot: &CoreRoomSnapshot,
    breakpoint: ConsoleBreakpoint,
) -> RightRailSection {
    let incomplete = snapshot
        .evidence
        .iter()
        .filter(|evidence| {
            evidence.status != EvidenceClosureState::Complete || !evidence.tracker_updated
        })
        .count();
    let mut rows = vec![row(
        "open evidence",
        incomplete.to_string(),
        state_from_count(incomplete),
        (incomplete > 0)
            .then(|| "complete Evidence Packet and tracker row before closure".to_owned()),
        None,
    )];
    rows.extend(
        snapshot
            .evidence
            .iter()
            .filter(|evidence| {
                evidence.status != EvidenceClosureState::Complete || !evidence.tracker_updated
            })
            .take(row_limit(breakpoint))
            .map(|evidence| {
                row(
                    evidence.work_order.clone(),
                    evidence_status_label(evidence.status),
                    Some(evidence_status(evidence.status, evidence.tracker_updated)),
                    missing_evidence_action(evidence),
                    evidence.rollback.clone(),
                )
            }),
    );
    RightRailSection {
        kind: RightRailSectionKind::Evidence,
        title: "Evidence".to_owned(),
        rows,
    }
}

fn sources_section(snapshot: &CoreRoomSnapshot, breakpoint: ConsoleBreakpoint) -> RightRailSection {
    let unhealthy = snapshot
        .sources
        .iter()
        .filter(|source| source.status != SourceHealthState::Pinned)
        .count();
    let mut rows = vec![row(
        "unhealthy",
        unhealthy.to_string(),
        state_from_count(unhealthy),
        (unhealthy > 0).then(|| "ask before refresh or trust/visibility changes".to_owned()),
        None,
    )];
    rows.extend(
        snapshot
            .sources
            .iter()
            .filter(|source| source.status != SourceHealthState::Pinned)
            .take(row_limit(breakpoint))
            .map(|source| {
                row(
                    source.source_id.clone(),
                    source_health_label(source.status),
                    Some(source_status(source.status)),
                    source.findings.first().cloned(),
                    source.related_work_orders.first().cloned(),
                )
            }),
    );
    RightRailSection {
        kind: RightRailSectionKind::Sources,
        title: "Sources".to_owned(),
        rows,
    }
}

fn alerts_section(snapshot: &CoreRoomSnapshot, breakpoint: ConsoleBreakpoint) -> RightRailSection {
    let mut alerts = overview_health_signals(snapshot);
    alerts.extend(snapshot.alerts.clone());
    let rows = alerts
        .into_iter()
        .filter(|alert| alert.severity != HealthSeverity::Ok)
        .take(row_limit(breakpoint))
        .map(|alert| {
            row(
                alert.id,
                alert.title,
                Some(alert_status(alert.severity)),
                alert.next_action,
                Some(alert.source),
            )
        })
        .collect();
    RightRailSection {
        kind: RightRailSectionKind::Alerts,
        title: "Alerts".to_owned(),
        rows,
    }
}

fn row(
    label: impl Into<String>,
    value: impl Into<String>,
    status: Option<StatusState>,
    action: Option<String>,
    source: Option<String>,
) -> RightRailRow {
    RightRailRow {
        label: label.into(),
        value: value.into(),
        status,
        action,
        source,
    }
}

fn row_limit(breakpoint: ConsoleBreakpoint) -> usize {
    match breakpoint {
        ConsoleBreakpoint::SubMinimum => 0,
        ConsoleBreakpoint::Compact120 => 3,
        ConsoleBreakpoint::Standard160 => 5,
        ConsoleBreakpoint::Wide220 => 8,
    }
}

fn state_from_count(count: usize) -> Option<StatusState> {
    (count > 0).then_some(StatusState::Warn)
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

fn dirty_state_label(state: DirtyState) -> &'static str {
    match state {
        DirtyState::Clean => "clean",
        DirtyState::Dirty => "dirty",
        DirtyState::Unknown => "not observed",
    }
}

fn changed_files_label(state: DirtyState) -> &'static str {
    match state {
        DirtyState::Clean => "none observed",
        DirtyState::Dirty => "present",
        DirtyState::Unknown => "not observed",
    }
}

fn dirty_state(state: DirtyState) -> StatusState {
    match state {
        DirtyState::Clean => StatusState::Ok,
        DirtyState::Dirty => StatusState::Warn,
        DirtyState::Unknown => StatusState::Unknown,
    }
}

fn role_state_label(state: RoleLaneState) -> &'static str {
    match state {
        RoleLaneState::Enabled => "enabled",
        RoleLaneState::Idle => "idle",
        RoleLaneState::Working => "working",
        RoleLaneState::Reviewing => "reviewing",
        RoleLaneState::Blocked => "blocked",
        RoleLaneState::WaitingUser => "waiting-user",
        RoleLaneState::WaitingApproval => "waiting-approval",
        RoleLaneState::StaleSession => "stale-session",
    }
}

fn role_status(
    state: RoleLaneState,
    waiting_approval: bool,
    permission_mode: Option<&str>,
) -> StatusState {
    if permission_mode == Some("bypass") || waiting_approval {
        return StatusState::Warn;
    }
    match state {
        RoleLaneState::Blocked | RoleLaneState::StaleSession => StatusState::Blocking,
        RoleLaneState::WaitingUser
        | RoleLaneState::WaitingApproval
        | RoleLaneState::Working
        | RoleLaneState::Reviewing => StatusState::Warn,
        RoleLaneState::Enabled | RoleLaneState::Idle => StatusState::Ok,
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

fn evidence_status(status: EvidenceClosureState, tracker_updated: bool) -> StatusState {
    if !tracker_updated {
        return StatusState::Blocking;
    }
    match status {
        EvidenceClosureState::Complete => StatusState::Ok,
        EvidenceClosureState::Incomplete | EvidenceClosureState::Unverified => StatusState::Warn,
        EvidenceClosureState::Missing => StatusState::Blocking,
    }
}

fn missing_evidence_action(evidence: &crate::console_snapshot::EvidenceSnapshot) -> Option<String> {
    if evidence.status == EvidenceClosureState::Complete && evidence.tracker_updated {
        None
    } else if evidence.missing_fields.is_empty() {
        Some("update tracker and verify evidence closure".to_owned())
    } else {
        Some(format!("missing: {}", evidence.missing_fields.join(", ")))
    }
}

fn source_health_label(status: SourceHealthState) -> &'static str {
    match status {
        SourceHealthState::Pinned => "pinned",
        SourceHealthState::Stale => "stale",
        SourceHealthState::Missing => "missing",
        SourceHealthState::TrustChanged => "trust-changed",
        SourceHealthState::VisibilityDenied => "visibility-denied",
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

fn alert_status(severity: HealthSeverity) -> StatusState {
    match severity {
        HealthSeverity::Ok => StatusState::Ok,
        HealthSeverity::Warn => StatusState::Warn,
        HealthSeverity::Blocking => StatusState::Blocking,
        HealthSeverity::Unknown => StatusState::Unknown,
    }
}
