//! Host-led WorkOrder model and GitHub Issue binding helpers.
//!
//! WorkOrders are project-level coordination records. They do not replace
//! GitHub Issues; they bind local CoreRoom state to the issue, gate thread,
//! branch, PR, tracker row, and evidence expectations that `@host` must manage.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::COREROOM_DIR;

/// Subdirectory inside `.coreroom/` that stores project WorkOrders.
pub const WORK_ORDERS_DIR: &str = "work-orders";

/// Current persisted WorkOrder schema version.
pub const WORK_ORDER_SCHEMA_VERSION: u32 = 1;

/// Host classification category that permits a WorkOrder draft.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum HostIntentClassification {
    /// Read-only or tiny low-risk work that stays inline.
    Tier0Inline,
    /// Persistent engineering work that needs issue, gate, evidence, and PR discipline.
    PersistentWorkorder,
    /// Product, architecture, or trust-boundary amendment work.
    ConstitutionAmendment,
    /// Release, audit, incident, compliance, or security-sensitive review.
    ReleaseAuditReview,
    /// Host does not have enough context to choose safely.
    InsufficientContext,
}

impl HostIntentClassification {
    /// Stable label used in fixtures and host-facing status.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Tier0Inline => "tier-0-inline",
            Self::PersistentWorkorder => "persistent-workorder",
            Self::ConstitutionAmendment => "constitution-amendment",
            Self::ReleaseAuditReview => "release-audit-review",
            Self::InsufficientContext => "insufficient-context",
        }
    }
}

/// Canonical WorkOrder lifecycle status.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum WorkOrderStatus {
    /// Host has drafted the work, but no persistent binding is confirmed yet.
    #[default]
    Draft,
    /// Host proposed the work and is waiting for user confirmation.
    Proposed,
    /// Work is confirmed and ready for an AI worker.
    Ready,
    /// Implementation or documentation work has started.
    InProgress,
    /// A PR or review is open.
    InReview,
    /// The PR merged, but release-level closure may still be pending.
    Merged,
    /// Work is blocked on missing context, failed validation, or user decision.
    Blocked,
    /// Work is fully closed in tracker/evidence terms.
    Closed,
}

impl WorkOrderStatus {
    /// Stable label used in persisted files and status cards.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Proposed => "proposed",
            Self::Ready => "ready",
            Self::InProgress => "in-progress",
            Self::InReview => "in-review",
            Self::Merged => "merged",
            Self::Blocked => "blocked",
            Self::Closed => "closed",
        }
    }
}

/// Evidence categories that a WorkOrder can require before closure.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum RequiredEvidence {
    /// Summary of files changed by the implementation.
    ChangedFiles,
    /// Commands, test results, or other validation output.
    Validation,
    /// SDLC gate ledger or gate artifact link.
    GateLedger,
    /// Specialist role reviews or authority decisions.
    RoleReviews,
    /// Pull request link and PR body evidence.
    PullRequest,
    /// Remaining risk or accepted-risk statement.
    Risks,
    /// Rollback plan.
    Rollback,
    /// Milestone tracker checkbox and Evidence Ledger row update.
    TrackerUpdate,
}

impl RequiredEvidence {
    /// Stable label used in persisted files and status cards.
    pub const fn label(self) -> &'static str {
        match self {
            Self::ChangedFiles => "changed-files",
            Self::Validation => "validation",
            Self::GateLedger => "gate-ledger",
            Self::RoleReviews => "role-reviews",
            Self::PullRequest => "pull-request",
            Self::Risks => "risks",
            Self::Rollback => "rollback",
            Self::TrackerUpdate => "tracker-update",
        }
    }
}

/// Input that `@host` uses to draft a WorkOrder after intent classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkOrderDraft {
    /// Stable WorkOrder id, usually `WO-<issue-number>`.
    pub id: String,
    /// Optional title. When absent, the first request line is used.
    pub title: Option<String>,
    /// User request or host-normalized objective.
    pub request: String,
    /// Prior host classification.
    pub classification: HostIntentClassification,
    /// Optional phase id or milestone name.
    pub phase: Option<String>,
    /// Optional epic or capability area.
    pub epic: Option<String>,
    /// Optional tracker issue number.
    pub tracker_issue: Option<u64>,
    /// Optional tracker checkbox text.
    pub tracker_checkbox: Option<String>,
    /// Acceptance criteria drafted by host.
    pub acceptance_criteria: Vec<String>,
    /// Evidence expected before the WorkOrder can close.
    pub required_evidence: Vec<RequiredEvidence>,
}

impl WorkOrderDraft {
    /// Create a minimal draft from a classified request.
    pub fn new(
        id: impl Into<String>,
        request: impl Into<String>,
        classification: HostIntentClassification,
    ) -> Self {
        Self {
            id: id.into(),
            title: None,
            request: request.into(),
            classification,
            phase: None,
            epic: None,
            tracker_issue: None,
            tracker_checkbox: None,
            acceptance_criteria: Vec::new(),
            required_evidence: default_required_evidence(),
        }
    }
}

/// Persisted WorkOrder record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkOrder {
    /// Schema version for forward-compatible parsing.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Stable WorkOrder id, usually `WO-<issue-number>`.
    pub id: String,
    /// Short human-readable title.
    pub title: String,
    /// Host-normalized objective or source request.
    pub objective: String,
    /// Bound GitHub Issue number, if confirmed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_issue: Option<u64>,
    /// Phase or milestone reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// Epic or capability area reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epic: Option<String>,
    /// Bound SDLC gate thread id, if one exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_thread: Option<String>,
    /// Implementation branch, if work has started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Pull request number, if one exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull_request: Option<u64>,
    /// Canonical lifecycle status.
    #[serde(default)]
    pub status: WorkOrderStatus,
    /// Acceptance criteria copied or normalized from the GitHub Issue.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acceptance_criteria: Vec<String>,
    /// Evidence required before closure.
    #[serde(
        default = "default_required_evidence",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub required_evidence: Vec<RequiredEvidence>,
    /// Tracker issue number, if this WorkOrder belongs to a milestone tracker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracker_issue: Option<u64>,
    /// Exact tracker checkbox text or stable row key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracker_checkbox: Option<String>,
}

impl WorkOrder {
    /// Draft a WorkOrder from a host-classified persistent request.
    pub fn draft_from_host_intake(draft: WorkOrderDraft) -> Result<Self> {
        if draft.classification != HostIntentClassification::PersistentWorkorder {
            bail!(
                "cannot draft WorkOrder from `{}` classification",
                draft.classification.label()
            );
        }

        let objective = draft.request.trim().to_owned();
        if objective.is_empty() {
            bail!("WorkOrder objective cannot be empty");
        }

        let required_evidence = if draft.required_evidence.is_empty() {
            default_required_evidence()
        } else {
            draft.required_evidence
        };

        let work_order = Self {
            schema_version: WORK_ORDER_SCHEMA_VERSION,
            id: draft.id,
            title: draft
                .title
                .unwrap_or_else(|| title_from_request(&objective)),
            objective,
            github_issue: None,
            phase: draft.phase,
            epic: draft.epic,
            gate_thread: None,
            branch: None,
            pull_request: None,
            status: WorkOrderStatus::Draft,
            acceptance_criteria: draft.acceptance_criteria,
            required_evidence,
            tracker_issue: draft.tracker_issue,
            tracker_checkbox: draft.tracker_checkbox,
        };
        work_order.validate()?;
        Ok(work_order)
    }

    /// Validate structural fields without consulting GitHub or gate ledgers.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != WORK_ORDER_SCHEMA_VERSION {
            bail!(
                "unsupported WorkOrder schemaVersion {}; expected {}",
                self.schema_version,
                WORK_ORDER_SCHEMA_VERSION
            );
        }
        validate_work_order_id(&self.id)?;
        ensure_nonempty("title", &self.title)?;
        ensure_nonempty("objective", &self.objective)?;
        ensure_positive_number("githubIssue", self.github_issue)?;
        ensure_positive_number("pullRequest", self.pull_request)?;
        ensure_positive_number("trackerIssue", self.tracker_issue)?;
        if self.acceptance_criteria.is_empty() {
            bail!("WorkOrder acceptanceCriteria cannot be empty");
        }
        if self.required_evidence.is_empty() {
            bail!("WorkOrder requiredEvidence cannot be empty");
        }
        if self.tracker_checkbox.is_some() && self.tracker_issue.is_none() {
            bail!("trackerCheckbox requires trackerIssue");
        }
        Ok(())
    }

    /// Return a confirmation-required plan for binding an existing GitHub Issue.
    pub fn plan_existing_issue_binding(issue: u64) -> Result<GitHubIssueBindingPlan> {
        GitHubIssueBindingPlan::bind_existing(issue)
    }

    /// Apply a previously confirmed GitHub Issue binding to local WorkOrder state.
    ///
    /// This only changes the local WorkOrder. It never edits the GitHub Issue
    /// body, labels, milestone, or comments.
    pub fn apply_confirmed_issue_binding(
        &mut self,
        binding: &ConfirmedGitHubIssueBinding,
    ) -> Result<()> {
        self.github_issue = Some(binding.plan.github_issue);
        self.validate()
    }

    /// Render a host-facing status card from local WorkOrder fields.
    pub fn render_status_card(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "WorkOrder {} - {}", self.id, self.title);
        let _ = writeln!(out, "status: {}", self.status.label());
        let _ = writeln!(out, "githubIssue: {}", format_issue(self.github_issue));
        let _ = writeln!(out, "phase: {}", format_optional(self.phase.as_deref()));
        let _ = writeln!(out, "epic: {}", format_optional(self.epic.as_deref()));
        let _ = writeln!(
            out,
            "gateThread: {}",
            format_optional(self.gate_thread.as_deref())
        );
        let _ = writeln!(out, "branch: {}", format_optional(self.branch.as_deref()));
        let _ = writeln!(out, "pullRequest: {}", format_issue(self.pull_request));
        let _ = writeln!(out, "trackerIssue: {}", format_issue(self.tracker_issue));
        let _ = writeln!(
            out,
            "trackerCheckbox: {}",
            format_optional(self.tracker_checkbox.as_deref())
        );
        let _ = writeln!(
            out,
            "acceptanceCriteria: {} item(s)",
            self.acceptance_criteria.len()
        );
        let evidence = self
            .required_evidence
            .iter()
            .map(|item| item.label())
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(out, "requiredEvidence: {evidence}");
        out
    }
}

/// Confirmation-required GitHub Issue binding plan.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitHubIssueBindingPlan {
    /// Existing GitHub Issue to bind.
    pub github_issue: u64,
    /// Binding action.
    pub action: GitHubIssueBindingAction,
    /// Host must ask the user before applying this plan.
    pub requires_confirmation: bool,
    /// Whether applying this plan mutates the GitHub Issue body.
    pub mutates_issue_body: bool,
}

impl GitHubIssueBindingPlan {
    /// Build a plan for binding an existing GitHub Issue.
    pub fn bind_existing(issue: u64) -> Result<Self> {
        if issue == 0 {
            bail!("githubIssue must be greater than zero");
        }
        Ok(Self {
            github_issue: issue,
            action: GitHubIssueBindingAction::BindExisting,
            requires_confirmation: true,
            mutates_issue_body: false,
        })
    }

    /// Convert this plan into a confirmed binding after explicit user approval.
    pub fn confirm(self, confirmed_by: impl Into<String>) -> Result<ConfirmedGitHubIssueBinding> {
        let confirmed_by = confirmed_by.into();
        ensure_nonempty("confirmedBy", &confirmed_by)?;
        Ok(ConfirmedGitHubIssueBinding {
            plan: self,
            confirmed_by,
        })
    }
}

/// GitHub Issue binding action.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum GitHubIssueBindingAction {
    /// Bind a known GitHub Issue to a local WorkOrder.
    BindExisting,
}

/// Confirmed GitHub Issue binding, ready to apply locally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmedGitHubIssueBinding {
    /// Original binding plan.
    pub plan: GitHubIssueBindingPlan,
    /// Actor who confirmed the binding.
    pub confirmed_by: String,
}

/// Save a WorkOrder under `.coreroom/work-orders/<id>.toml`.
pub fn save_work_order(project_root: &Path, work_order: &WorkOrder) -> Result<PathBuf> {
    work_order.validate()?;
    let path = work_order_path(project_root, &work_order.id)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let content = toml::to_string_pretty(work_order).context("serializing WorkOrder")?;
    std::fs::write(&path, ensure_trailing_newline(&content))
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

/// Load and validate a WorkOrder TOML file.
pub fn load_work_order(path: &Path) -> Result<WorkOrder> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let work_order: WorkOrder =
        toml::from_str(&content).with_context(|| format!("parsing {}", path.display()))?;
    work_order.validate()?;
    Ok(work_order)
}

/// Return the canonical path for a WorkOrder id.
pub fn work_order_path(project_root: &Path, id: &str) -> Result<PathBuf> {
    validate_work_order_id(id)?;
    Ok(project_root
        .join(COREROOM_DIR)
        .join(WORK_ORDERS_DIR)
        .join(format!("{id}.toml")))
}

fn default_schema_version() -> u32 {
    WORK_ORDER_SCHEMA_VERSION
}

fn default_required_evidence() -> Vec<RequiredEvidence> {
    vec![
        RequiredEvidence::ChangedFiles,
        RequiredEvidence::Validation,
        RequiredEvidence::Risks,
        RequiredEvidence::Rollback,
        RequiredEvidence::TrackerUpdate,
    ]
}

fn validate_work_order_id(id: &str) -> Result<()> {
    let Some(number) = id.strip_prefix("WO-") else {
        bail!("WorkOrder id `{id}` must start with `WO-`");
    };
    if number.is_empty() || !number.chars().all(|ch| ch.is_ascii_digit()) {
        bail!("WorkOrder id `{id}` must use `WO-<digits>`");
    }
    Ok(())
}

fn ensure_nonempty(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{field} cannot be empty");
    }
    Ok(())
}

fn ensure_positive_number(field: &str, value: Option<u64>) -> Result<()> {
    if value == Some(0) {
        bail!("{field} must be greater than zero");
    }
    Ok(())
}

fn title_from_request(request: &str) -> String {
    let first_line = request
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("Untitled WorkOrder");
    truncate_chars(first_line, 120)
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_owned();
    }
    let mut out = input
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    out.push_str("...");
    out
}

fn format_issue(value: Option<u64>) -> String {
    value.map_or_else(|| "unbound".to_owned(), |issue| format!("#{issue}"))
}

fn format_optional(value: Option<&str>) -> String {
    value.map_or_else(|| "unbound".to_owned(), ToOwned::to_owned)
}

fn ensure_trailing_newline(input: &str) -> String {
    if input.ends_with('\n') {
        input.to_owned()
    } else {
        format!("{input}\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> WorkOrderDraft {
        let mut draft = WorkOrderDraft::new(
            "WO-0207",
            "Bind WorkOrder to GitHub Issue\nKeep issue body unchanged.",
            HostIntentClassification::PersistentWorkorder,
        );
        draft.phase = Some("v0.6.0 - Engineering Control Room".to_owned());
        draft.epic = Some("WorkOrder / GitHub Binding".to_owned());
        draft.tracker_issue = Some(202);
        draft.tracker_checkbox = Some("#207 - WorkOrder model and GitHub binding".to_owned());
        draft.acceptance_criteria = vec![
            "WorkOrder has canonical status".to_owned(),
            "Binding existing issue does not mutate issue body".to_owned(),
        ];
        draft
    }

    #[test]
    fn draft_requires_persistent_workorder_classification() {
        let mut draft = draft();
        draft.classification = HostIntentClassification::Tier0Inline;

        let err = WorkOrder::draft_from_host_intake(draft).expect_err("draft blocked");
        assert!(err.to_string().contains("tier-0-inline"));
    }

    #[test]
    fn confirmed_existing_issue_binding_changes_only_local_state() {
        let mut work_order = WorkOrder::draft_from_host_intake(draft()).expect("draft");
        let plan = WorkOrder::plan_existing_issue_binding(207).expect("plan");

        assert!(plan.requires_confirmation);
        assert!(!plan.mutates_issue_body);
        assert_eq!(plan.action, GitHubIssueBindingAction::BindExisting);

        let confirmed = plan.confirm("user").expect("confirmed");
        work_order
            .apply_confirmed_issue_binding(&confirmed)
            .expect("apply");

        assert_eq!(work_order.github_issue, Some(207));
        assert_eq!(work_order.status, WorkOrderStatus::Draft);
    }

    #[test]
    fn status_card_lists_bindings_and_evidence() {
        let mut work_order = WorkOrder::draft_from_host_intake(draft()).expect("draft");
        work_order.github_issue = Some(207);
        work_order.gate_thread = Some("thread-207".to_owned());
        work_order.branch = Some("feat/v0.6-207-workorder-github-binding".to_owned());
        work_order.pull_request = Some(223);
        work_order.status = WorkOrderStatus::InReview;

        let status = work_order.render_status_card();

        assert!(status.contains("WorkOrder WO-0207"));
        assert!(status.contains("status: in-review"));
        assert!(status.contains("githubIssue: #207"));
        assert!(status.contains("pullRequest: #223"));
        assert!(status.contains("requiredEvidence: changed-files, validation"));
    }

    #[test]
    fn invalid_status_values_fail_deserialization() {
        let content = r#"
schemaVersion = 1
id = "WO-0207"
title = "Bad status"
objective = "Prove canonical status parsing."
status = "almost-done"
acceptanceCriteria = ["AC"]
requiredEvidence = ["validation"]
"#;

        let err = toml::from_str::<WorkOrder>(content).expect_err("invalid status");
        assert!(err.to_string().contains("unknown variant"));
    }
}
