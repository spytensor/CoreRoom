//! Console data-plane snapshot for the future CoreRoom full-screen view.
//!
//! v0.8 defines this as a structural contract only. Renderers, live polling,
//! reducers, and host actions consume or extend the snapshot in later issues.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// Current console snapshot schema version.
pub const CONSOLE_SNAPSHOT_SCHEMA_VERSION: u32 = 1;

/// Complete state packet for a CoreRoom console render pass.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CoreRoomSnapshot {
    /// Schema version.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Project/repository identity.
    pub project: ProjectIdentity,
    /// Runtime and role state.
    pub runtime: RuntimeSnapshot,
    /// Public conversation and internal delegation counters.
    pub conversation: ConversationSnapshot,
    /// WorkOrder rows visible to the console.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub work: Vec<WorkSnapshot>,
    /// Gate state rows.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gates: Vec<GateSnapshot>,
    /// Evidence closure rows.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<EvidenceSnapshot>,
    /// Source health rows.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<SourceHealthSnapshot>,
    /// GitHub aggregate facts.
    pub github: GitHubSnapshot,
    /// Actionable health signals.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alerts: Vec<HealthSignal>,
    /// Renderer-independent layout hints.
    pub layout: LayoutHints,
}

impl CoreRoomSnapshot {
    /// Validate shape and minimum useful console facts.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != CONSOLE_SNAPSHOT_SCHEMA_VERSION {
            bail!(
                "unsupported CoreRoomSnapshot schemaVersion {}; expected {}",
                self.schema_version,
                CONSOLE_SNAPSHOT_SCHEMA_VERSION
            );
        }
        self.project.validate()?;
        self.runtime.validate()?;
        self.conversation.validate()?;
        self.github.validate()?;
        self.layout.validate()?;
        if self.work.is_empty() {
            bail!("CoreRoomSnapshot work cannot be empty");
        }
        for work in &self.work {
            work.validate()?;
        }
        for gate in &self.gates {
            gate.validate()?;
        }
        for evidence in &self.evidence {
            evidence.validate()?;
        }
        for source in &self.sources {
            source.validate()?;
        }
        for alert in &self.alerts {
            alert.validate()?;
        }
        Ok(())
    }
}

/// Project/repository identity for the console header.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectIdentity {
    /// Product or project name.
    pub project: String,
    /// Repository in owner/name form when known.
    pub repository: String,
    /// Remote URL or canonical web URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    /// Current branch or worktree branch.
    pub branch: String,
    /// Current head SHA.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_sha: Option<String>,
    /// Worktree dirty state.
    pub dirty_state: DirtyState,
    /// CoreRoom version.
    pub version: String,
    /// Active phase or milestone.
    pub active_phase: String,
    /// Active tracker issue.
    pub tracker_issue: u64,
}

impl ProjectIdentity {
    fn validate(&self) -> Result<()> {
        ensure_nonempty("project.project", &self.project)?;
        ensure_nonempty("project.repository", &self.repository)?;
        ensure_nonempty("project.branch", &self.branch)?;
        ensure_nonempty("project.version", &self.version)?;
        ensure_nonempty("project.activePhase", &self.active_phase)?;
        if self.tracker_issue == 0 {
            bail!("project.trackerIssue must be non-zero");
        }
        Ok(())
    }
}

/// Worktree dirty state.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum DirtyState {
    /// No known local changes.
    Clean,
    /// Local changes exist.
    Dirty,
    /// Dirty state was not observed.
    Unknown,
}

/// Runtime and role state for the current room.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSnapshot {
    /// Current room/session id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_id: Option<String>,
    /// Configured host role.
    pub host_role: String,
    /// Freshness of the current engine session bindings.
    pub session_state: SessionFreshness,
    /// Current permission mode summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    /// Enabled roles.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<RoleRuntimeSnapshot>,
    /// Active role if a turn is inflight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_role: Option<String>,
    /// Whether any role is waiting for approval.
    #[serde(default)]
    pub waiting_approval: bool,
}

impl RuntimeSnapshot {
    fn validate(&self) -> Result<()> {
        ensure_nonempty("runtime.hostRole", &self.host_role)?;
        if self.roles.is_empty() {
            bail!("runtime.roles cannot be empty");
        }
        for role in &self.roles {
            role.validate()?;
        }
        Ok(())
    }
}

/// Freshness of the current room/session binding.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum SessionFreshness {
    /// The room started fresh in this process.
    Fresh,
    /// The room resumed a known prior session.
    Resumed,
    /// The room resumed but the source session may be stale.
    Stale,
    /// Session freshness was not observed.
    Unknown,
}

/// One role lane in the console.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RoleRuntimeSnapshot {
    /// Role name without `@`.
    pub role: String,
    /// Engine id such as `cc`, `codex`, or `gemini`.
    pub engine: String,
    /// Model label when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Role lane state.
    pub state: RoleLaneState,
    /// WorkOrder currently associated with the role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_work_order: Option<String>,
    /// Gate phase currently associated with the role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_gate_phase: Option<String>,
    /// Human-readable last activity marker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity: Option<String>,
}

impl RoleRuntimeSnapshot {
    fn validate(&self) -> Result<()> {
        ensure_nonempty("role.role", &self.role)?;
        ensure_nonempty("role.engine", &self.engine)?;
        if let Some(work_order) = &self.current_work_order {
            ensure_nonempty("role.currentWorkOrder", work_order)?;
        }
        Ok(())
    }
}

/// Role lane state.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum RoleLaneState {
    /// Role is configured but not currently doing work.
    Idle,
    /// Role has an active turn.
    Working,
    /// Role is reviewing a plan or work item.
    Reviewing,
    /// Role is blocked.
    Blocked,
    /// Role is waiting for user input.
    WaitingUser,
    /// Role is waiting for a tool approval.
    WaitingApproval,
    /// Role resumed an old session and may be stale.
    StaleSession,
}

/// Conversation slice for the console center panel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSnapshot {
    /// Public turns visible in the center conversation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub public_turns: Vec<ConversationTurn>,
    /// Count of internal delegations hidden from the public transcript.
    #[serde(default)]
    pub internal_delegation_count: u32,
}

impl ConversationSnapshot {
    fn validate(&self) -> Result<()> {
        for turn in &self.public_turns {
            turn.validate()?;
        }
        Ok(())
    }
}

/// One visible conversation turn.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConversationTurn {
    /// Speaker label such as `user`, `host`, or a role.
    pub speaker: String,
    /// Turn body.
    pub body: String,
    /// Visibility class.
    pub visibility: ConversationVisibility,
}

impl ConversationTurn {
    fn validate(&self) -> Result<()> {
        ensure_nonempty("conversation.speaker", &self.speaker)?;
        ensure_nonempty("conversation.body", &self.body)
    }
}

/// Conversation visibility class.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum ConversationVisibility {
    /// Main user-facing transcript.
    PublicTranscript,
    /// Host-managed internal delegation.
    InternalDelegation,
    /// Side-rail status summary.
    SideRail,
    /// Debug/audit log.
    DebugLog,
}

/// One WorkOrder row in the console.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkSnapshot {
    /// WorkOrder id.
    pub id: String,
    /// Human title.
    pub title: String,
    /// Phase or milestone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// Epic or capability area.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epic: Option<String>,
    /// Bound GitHub Issue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_issue: Option<u64>,
    /// Branch name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Pull request number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull_request: Option<u64>,
    /// CI state label.
    pub ci_state: StatusState,
    /// Evidence state label.
    pub evidence_state: StatusState,
    /// Tracker state label.
    pub tracker_state: StatusState,
    /// Work lifecycle.
    pub lifecycle: WorkLifecycle,
    /// Source citations related to this work.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_citations: Vec<String>,
}

impl WorkSnapshot {
    fn validate(&self) -> Result<()> {
        ensure_work_order_id(&self.id)?;
        ensure_nonempty("work.title", &self.title)
    }
}

/// Generic console state.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum StatusState {
    /// State is healthy or satisfied.
    Ok,
    /// State requires attention but is not blocking.
    Warn,
    /// State blocks a completion claim or next step.
    Blocking,
    /// State was not observed.
    Unknown,
}

/// Work lifecycle for console rows.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum WorkLifecycle {
    /// Work has not started.
    NotStarted,
    /// Work is ready to pick up.
    Ready,
    /// Work is on a branch.
    InProgress,
    /// Work has an open review/PR.
    InReview,
    /// Required checks failed.
    FailedCi,
    /// Work is blocked.
    Blocked,
    /// Implementation merged but tracker/evidence is stale.
    MergedTrackerStale,
    /// Work is fully closed.
    Closed,
}

/// Gate state for a WorkOrder.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GateSnapshot {
    /// WorkOrder id.
    pub work_order: String,
    /// Current gate phase.
    pub current_phase: String,
    /// Blocked reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    /// Missing role reviews.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_reviews: Vec<String>,
    /// Stale plan SHA if detected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale_plan_sha: Option<String>,
    /// Whether signoff is ready.
    #[serde(default)]
    pub signoff_ready: bool,
}

impl GateSnapshot {
    fn validate(&self) -> Result<()> {
        ensure_work_order_id(&self.work_order)?;
        ensure_nonempty("gate.currentPhase", &self.current_phase)
    }
}

/// Evidence closure state for a WorkOrder.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceSnapshot {
    /// WorkOrder id.
    pub work_order: String,
    /// Evidence status.
    pub status: EvidenceClosureState,
    /// Missing evidence fields.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_fields: Vec<String>,
    /// Explicitly unverified items.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unverified_items: Vec<String>,
    /// Rollback plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollback: Option<String>,
    /// Whether tracker row/ledger is updated.
    #[serde(default)]
    pub tracker_updated: bool,
}

impl EvidenceSnapshot {
    fn validate(&self) -> Result<()> {
        ensure_work_order_id(&self.work_order)
    }
}

/// Evidence closure status.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceClosureState {
    /// Evidence is complete.
    Complete,
    /// Evidence is present but incomplete.
    Incomplete,
    /// Evidence is missing.
    Missing,
    /// Evidence contains unverified items.
    Unverified,
}

/// Source health row for the console.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceHealthSnapshot {
    /// Source id.
    pub source_id: String,
    /// Source health.
    pub status: SourceHealthState,
    /// Trust level label.
    pub trust_level: String,
    /// Roles that may see this source.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub visible_roles: Vec<String>,
    /// Source findings.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<String>,
    /// Related WorkOrders.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_work_orders: Vec<String>,
}

impl SourceHealthSnapshot {
    fn validate(&self) -> Result<()> {
        ensure_nonempty("source.sourceId", &self.source_id)?;
        ensure_nonempty("source.trustLevel", &self.trust_level)
    }
}

/// Source health state.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum SourceHealthState {
    /// Source is healthy.
    Pinned,
    /// Source pin drifted.
    Stale,
    /// Source is missing.
    Missing,
    /// Trust level changed.
    TrustChanged,
    /// Visibility denied.
    VisibilityDenied,
}

/// GitHub aggregate facts for the console header/rail.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitHubSnapshot {
    /// Repository in owner/name form.
    pub repository: String,
    /// Tracker issue.
    pub tracker_issue: u64,
    /// Open issue count relevant to the active milestone.
    #[serde(default)]
    pub open_issues: u32,
    /// Open PR count relevant to the active milestone.
    #[serde(default)]
    pub open_pull_requests: u32,
    /// Failing check count.
    #[serde(default)]
    pub failing_checks: u32,
}

impl GitHubSnapshot {
    fn validate(&self) -> Result<()> {
        ensure_nonempty("github.repository", &self.repository)?;
        if self.tracker_issue == 0 {
            bail!("github.trackerIssue must be non-zero");
        }
        Ok(())
    }
}

/// Actionable console alert.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HealthSignal {
    /// Stable signal id.
    pub id: String,
    /// Severity.
    pub severity: HealthSeverity,
    /// Human title.
    pub title: String,
    /// Structural source label.
    pub source: String,
    /// Recommended next action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_action: Option<String>,
}

impl HealthSignal {
    fn validate(&self) -> Result<()> {
        ensure_nonempty("alert.id", &self.id)?;
        ensure_nonempty("alert.title", &self.title)?;
        ensure_nonempty("alert.source", &self.source)
    }
}

/// Health signal severity.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum HealthSeverity {
    /// Healthy signal.
    Ok,
    /// Warning signal.
    Warn,
    /// Blocking signal.
    Blocking,
    /// Freshness or state is unknown.
    Unknown,
}

/// Renderer-independent layout hints.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LayoutHints {
    /// Primary pane id.
    pub primary_pane: String,
    /// Minimum supported terminal columns.
    pub min_columns: u16,
    /// Preferred terminal columns.
    pub preferred_columns: u16,
    /// Panes collapsed at the minimum layout.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collapsed_panes: Vec<String>,
}

impl LayoutHints {
    fn validate(&self) -> Result<()> {
        ensure_nonempty("layout.primaryPane", &self.primary_pane)?;
        if self.min_columns == 0 {
            bail!("layout.minColumns must be non-zero");
        }
        if self.preferred_columns < self.min_columns {
            bail!("layout.preferredColumns must be >= minColumns");
        }
        Ok(())
    }
}

fn default_schema_version() -> u32 {
    CONSOLE_SNAPSHOT_SCHEMA_VERSION
}

fn ensure_nonempty(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{field} cannot be empty");
    }
    Ok(())
}

fn ensure_work_order_id(value: &str) -> Result<()> {
    ensure_nonempty("workOrder", value)?;
    if !value.starts_with("WO-") {
        bail!("WorkOrder id `{value}` must start with `WO-`");
    }
    Ok(())
}
