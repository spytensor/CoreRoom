//! Project and release status rollups for `@host`.
//!
//! The rollup consumes structural facts from WorkOrders, GitHub, CI, Evidence
//! Packets, tracker rows, ContextPacks, and source graph findings. It renders a
//! user-facing status card without treating narrative model output as proof.

use std::fmt::Write as _;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::github_status::{
    derive_github_work_order_status, EvidencePacketState, GitHubWorkOrderFacts, PullRequestState,
    WorkOrderLifecycle,
};
use crate::source_graph::{
    SourceGraphEvidenceCitation, SourceGraphFinding, SourceGraphFindingKind,
};

/// Current project status schema version.
pub const PROJECT_STATUS_SCHEMA_VERSION: u32 = 1;

/// Input facts for a project status rollup.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectStatusInput {
    /// Schema version.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Project or product name.
    pub project: String,
    /// Milestone, phase, or release name.
    pub milestone: String,
    /// WorkOrders tracked by this rollup.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub work_orders: Vec<ProjectWorkOrderInput>,
    /// Source graph findings that affect release confidence.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_findings: Vec<SourceGraphFinding>,
    /// Known risks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub risks: Vec<ProjectRisk>,
}

impl ProjectStatusInput {
    /// Validate shape.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != PROJECT_STATUS_SCHEMA_VERSION {
            bail!(
                "unsupported Project Status schemaVersion {}; expected {}",
                self.schema_version,
                PROJECT_STATUS_SCHEMA_VERSION
            );
        }
        ensure_nonempty("project", &self.project)?;
        ensure_nonempty("milestone", &self.milestone)?;
        if self.work_orders.is_empty() {
            bail!("ProjectStatus workOrders cannot be empty");
        }
        for work_order in &self.work_orders {
            work_order.validate()?;
        }
        for risk in &self.risks {
            risk.validate()?;
        }
        Ok(())
    }
}

/// One WorkOrder's rollup input facts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectWorkOrderInput {
    /// WorkOrder id.
    pub id: String,
    /// WorkOrder title.
    pub title: String,
    /// GitHub Issue/PR/CI/evidence/tracker facts.
    pub github: GitHubWorkOrderFacts,
    /// ContextPack id used for this work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_pack: Option<String>,
    /// Source citations used for this work.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_citations: Vec<SourceGraphEvidenceCitation>,
    /// Human blocker, if the work is waiting for user input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human_blocker: Option<String>,
}

impl ProjectWorkOrderInput {
    /// Validate shape.
    pub fn validate(&self) -> Result<()> {
        validate_work_order_id(&self.id)?;
        ensure_nonempty("title", &self.title)?;
        if let Some(context_pack) = &self.context_pack {
            ensure_nonempty("contextPack", context_pack)?;
        }
        if let Some(blocker) = &self.human_blocker {
            ensure_nonempty("humanBlocker", blocker)?;
        }
        Ok(())
    }
}

/// Known project risk.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRisk {
    /// Risk level, such as `low`, `medium`, `high`, or `critical`.
    pub level: String,
    /// Risk description.
    pub description: String,
    /// Evidence citation.
    pub citation: String,
}

impl ProjectRisk {
    /// Validate shape.
    pub fn validate(&self) -> Result<()> {
        ensure_nonempty("risk.level", &self.level)?;
        ensure_nonempty("risk.description", &self.description)?;
        ensure_nonempty("risk.citation", &self.citation)?;
        Ok(())
    }

    fn is_high_or_critical(&self) -> bool {
        matches!(self.level.as_str(), "high" | "critical")
    }
}

/// Rollup WorkOrder state for user-facing cards.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum ProjectWorkState {
    /// Work has not started.
    NotStarted,
    /// Work is ready to be picked up.
    Ready,
    /// Work is in progress on a branch.
    InProgress,
    /// Work is in PR/review.
    InReview,
    /// CI failed.
    FailedCi,
    /// Work is blocked.
    Blocked,
    /// Implementation merged, but tracker/evidence closure is stale.
    ImplementationCompleteTrackerIncomplete,
    /// PR merged but evidence is not complete enough for closure.
    Merged,
    /// Work is fully closed.
    Closed,
}

impl ProjectWorkState {
    /// Stable label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::NotStarted => "not-started",
            Self::Ready => "ready",
            Self::InProgress => "in-progress",
            Self::InReview => "in-review",
            Self::FailedCi => "failed-ci",
            Self::Blocked => "blocked",
            Self::ImplementationCompleteTrackerIncomplete => {
                "implementation-complete-tracker-incomplete"
            }
            Self::Merged => "merged",
            Self::Closed => "closed",
        }
    }

    const fn is_open(self) -> bool {
        !matches!(self, Self::Closed)
    }
}

/// Release checkpoint decision.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum ReleaseDecision {
    /// Ready to release.
    ReadyToRelease,
    /// Continue normal project work.
    ContinueWork,
    /// Blocked until the user provides input.
    BlockedWaitingHumanInput,
    /// Unsafe to claim done.
    UnsafeToClaimDone,
}

impl ReleaseDecision {
    /// Stable label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::ReadyToRelease => "ready-to-release",
            Self::ContinueWork => "continue-work",
            Self::BlockedWaitingHumanInput => "blocked-waiting-human-input",
            Self::UnsafeToClaimDone => "unsafe-to-claim-done",
        }
    }
}

/// Derived WorkOrder status row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectWorkOrderStatus {
    /// WorkOrder id.
    pub id: String,
    /// WorkOrder title.
    pub title: String,
    /// Derived rollup state.
    pub state: ProjectWorkState,
    /// GitHub lifecycle label.
    pub github_lifecycle: WorkOrderLifecycle,
    /// Evidence citations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub citations: Vec<String>,
    /// Findings or blockers for this WorkOrder.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<String>,
}

/// Derived project status card.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectStatusCard {
    /// Project name.
    pub project: String,
    /// Milestone/phase/release.
    pub milestone: String,
    /// WorkOrder status rows.
    pub work_orders: Vec<ProjectWorkOrderStatus>,
    /// Known source findings.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_findings: Vec<SourceGraphFinding>,
    /// Known risks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub risks: Vec<ProjectRisk>,
    /// Release decision.
    pub decision: ReleaseDecision,
    /// Whether release readiness can be claimed.
    pub release_ready: bool,
    /// Blockers preventing release or completion claim.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<String>,
}

impl ProjectStatusCard {
    /// Render a host-facing status card suitable for conversation or PR notes.
    pub fn render_host_summary(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "Project: {}", self.project);
        let _ = writeln!(out, "Milestone: {}", self.milestone);
        let _ = writeln!(out, "Decision: {}", self.decision.label());
        let _ = writeln!(out, "Release ready: {}", self.release_ready);
        let _ = writeln!(out);

        write_work_section(
            &mut out,
            "Open work",
            self.work_orders
                .iter()
                .filter(|work| work.state.is_open())
                .map(format_work_row),
        );
        write_work_section(
            &mut out,
            "Active PRs",
            self.work_orders
                .iter()
                .filter(|work| work.state == ProjectWorkState::InReview)
                .map(format_work_row),
        );
        write_work_section(
            &mut out,
            "Blocked work",
            self.work_orders
                .iter()
                .filter(|work| work.state == ProjectWorkState::Blocked)
                .map(format_work_row),
        );
        write_work_section(
            &mut out,
            "Failed CI",
            self.work_orders
                .iter()
                .filter(|work| work.state == ProjectWorkState::FailedCi)
                .map(format_work_row),
        );
        write_work_section(
            &mut out,
            "Stale tracker rows",
            self.work_orders
                .iter()
                .filter(|work| {
                    work.state == ProjectWorkState::ImplementationCompleteTrackerIncomplete
                })
                .map(format_work_row),
        );
        write_work_section(
            &mut out,
            "Closed work",
            self.work_orders
                .iter()
                .filter(|work| work.state == ProjectWorkState::Closed)
                .map(format_work_row),
        );

        write_work_section(
            &mut out,
            "Stale sources",
            self.source_findings.iter().map(|finding| {
                format!(
                    "{} {} - {}",
                    finding.source_id,
                    finding.kind.label(),
                    finding.message
                )
            }),
        );
        write_work_section(
            &mut out,
            "Risks",
            self.risks
                .iter()
                .map(|risk| format!("{} - {} ({})", risk.level, risk.description, risk.citation)),
        );
        write_work_section(&mut out, "Blockers", self.blockers.iter().cloned());
        out
    }
}

/// Build a project status card from structural input facts.
pub fn build_project_status(input: ProjectStatusInput) -> Result<ProjectStatusCard> {
    input.validate()?;
    let mut blockers = Vec::new();
    let work_orders = input
        .work_orders
        .iter()
        .map(|work| {
            let github = derive_github_work_order_status(&work.github);
            let state = derive_project_work_state(work, github.lifecycle);
            let mut citations = github.citations;
            if let Some(context_pack) = &work.context_pack {
                citations.push(format!("contextPack:{context_pack}"));
            }
            for citation in &work.source_citations {
                citations.push(format!("source:{}:{}", citation.source_id, citation.pin));
                for path in &citation.graph_paths {
                    citations.push(format!("graphPath:{path}"));
                }
            }
            let mut findings = github.findings;
            if let Some(blocker) = &work.human_blocker {
                findings.push(format!("human input required: {blocker}"));
            }
            Ok(ProjectWorkOrderStatus {
                id: work.id.clone(),
                title: work.title.clone(),
                state,
                github_lifecycle: github.lifecycle,
                citations,
                findings,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    for work in &work_orders {
        match work.state {
            ProjectWorkState::FailedCi => {
                blockers.push(format!("{} has failed CI", work.id));
            }
            ProjectWorkState::Blocked => {
                blockers.push(format!("{} is blocked", work.id));
            }
            ProjectWorkState::ImplementationCompleteTrackerIncomplete => {
                blockers.push(format!(
                    "{} implementation complete, tracker incomplete",
                    work.id
                ));
            }
            ProjectWorkState::Merged => {
                blockers.push(format!(
                    "{} is merged but evidence closure is incomplete",
                    work.id
                ));
            }
            ProjectWorkState::NotStarted
            | ProjectWorkState::Ready
            | ProjectWorkState::InProgress
            | ProjectWorkState::InReview => {
                blockers.push(format!("{} is {}", work.id, work.state.label()));
            }
            ProjectWorkState::Closed => {}
        }
    }
    for finding in &input.source_findings {
        blockers.push(format!(
            "source {} has {}",
            finding.source_id,
            finding.kind.label()
        ));
    }
    for risk in input.risks.iter().filter(|risk| risk.is_high_or_critical()) {
        blockers.push(format!("{} risk: {}", risk.level, risk.description));
    }

    let decision = release_decision(&work_orders, &input.source_findings, &input.risks);
    let release_ready = decision == ReleaseDecision::ReadyToRelease;

    Ok(ProjectStatusCard {
        project: input.project,
        milestone: input.milestone,
        work_orders,
        source_findings: input.source_findings,
        risks: input.risks,
        decision,
        release_ready,
        blockers,
    })
}

fn derive_project_work_state(
    work: &ProjectWorkOrderInput,
    lifecycle: WorkOrderLifecycle,
) -> ProjectWorkState {
    if work.human_blocker.is_some() {
        return ProjectWorkState::Blocked;
    }
    match lifecycle {
        WorkOrderLifecycle::NotStarted => ProjectWorkState::NotStarted,
        WorkOrderLifecycle::Ready => ProjectWorkState::Ready,
        WorkOrderLifecycle::InProgress => ProjectWorkState::InProgress,
        WorkOrderLifecycle::InReview => ProjectWorkState::InReview,
        WorkOrderLifecycle::FailedCi => ProjectWorkState::FailedCi,
        WorkOrderLifecycle::Blocked => ProjectWorkState::Blocked,
        WorkOrderLifecycle::MergedTrackerStale => {
            if work.github.evidence == EvidencePacketState::Complete {
                ProjectWorkState::ImplementationCompleteTrackerIncomplete
            } else if work
                .github
                .pull_request
                .as_ref()
                .is_some_and(|pr| pr.state == PullRequestState::Merged)
            {
                ProjectWorkState::Merged
            } else {
                ProjectWorkState::ImplementationCompleteTrackerIncomplete
            }
        }
        WorkOrderLifecycle::Closed => ProjectWorkState::Closed,
    }
}

fn release_decision(
    work_orders: &[ProjectWorkOrderStatus],
    source_findings: &[SourceGraphFinding],
    risks: &[ProjectRisk],
) -> ReleaseDecision {
    if work_orders
        .iter()
        .any(|work| work.state == ProjectWorkState::FailedCi)
        || work_orders
            .iter()
            .any(|work| work.state == ProjectWorkState::ImplementationCompleteTrackerIncomplete)
        || source_findings
            .iter()
            .any(|finding| is_release_blocking_source_finding(finding.kind))
        || risks.iter().any(ProjectRisk::is_high_or_critical)
    {
        return ReleaseDecision::UnsafeToClaimDone;
    }
    if work_orders
        .iter()
        .any(|work| work.state == ProjectWorkState::Blocked)
    {
        return ReleaseDecision::BlockedWaitingHumanInput;
    }
    if work_orders
        .iter()
        .all(|work| work.state == ProjectWorkState::Closed)
    {
        ReleaseDecision::ReadyToRelease
    } else {
        ReleaseDecision::ContinueWork
    }
}

fn is_release_blocking_source_finding(kind: SourceGraphFindingKind) -> bool {
    matches!(
        kind,
        SourceGraphFindingKind::MissingSource
            | SourceGraphFindingKind::CommitChanged
            | SourceGraphFindingKind::FileHashChanged
            | SourceGraphFindingKind::UrlSnapshotStale
            | SourceGraphFindingKind::TrustChanged
            | SourceGraphFindingKind::VisibilityChanged
            | SourceGraphFindingKind::VisibilityDenied
    )
}

fn format_work_row(work: &ProjectWorkOrderStatus) -> String {
    let citations = if work.citations.is_empty() {
        "no citations".to_owned()
    } else {
        work.citations.join("; ")
    };
    let findings = if work.findings.is_empty() {
        "no findings".to_owned()
    } else {
        work.findings.join("; ")
    };
    format!(
        "{} {} - {} - citations: {} - findings: {}",
        work.id,
        work.title,
        work.state.label(),
        citations,
        findings
    )
}

fn write_work_section<I>(out: &mut String, title: &str, items: I)
where
    I: IntoIterator<Item = String>,
{
    let items = items.into_iter().collect::<Vec<_>>();
    let _ = writeln!(out, "## {title}");
    if items.is_empty() {
        let _ = writeln!(out, "- none");
    } else {
        for item in items {
            let _ = writeln!(out, "- {item}");
        }
    }
    let _ = writeln!(out);
}

fn validate_work_order_id(id: &str) -> Result<()> {
    ensure_nonempty("id", id)?;
    if !id.starts_with("WO-") || !id[3..].chars().all(|ch| ch.is_ascii_digit()) {
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

fn default_schema_version() -> u32 {
    PROJECT_STATUS_SCHEMA_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github_status::{CheckFacts, CheckState, GitHubIssueState, PullRequestFacts};
    use crate::tracker::TrackerEntryState;

    fn closed_work_order(id: &str) -> ProjectWorkOrderInput {
        ProjectWorkOrderInput {
            id: id.to_owned(),
            title: "Closed work".to_owned(),
            github: GitHubWorkOrderFacts {
                issue: 217,
                issue_state: GitHubIssueState::Closed,
                labels: Vec::new(),
                branch: Some("feat/closed".to_owned()),
                pull_request: Some(PullRequestFacts {
                    number: 240,
                    state: PullRequestState::Merged,
                }),
                checks: vec![CheckFacts {
                    name: "test".to_owned(),
                    state: CheckState::Pass,
                    url: None,
                }],
                tracker: TrackerEntryState {
                    checkbox_checked: true,
                    ledger_status: Some("merged".to_owned()),
                    ledger_tracker_updated: Some(true),
                },
                evidence: EvidencePacketState::Complete,
                evidence_packet: Some(id.to_owned()),
                blocker: None,
            },
            context_pack: Some(format!("CTX-{id}")),
            source_citations: Vec::new(),
            human_blocker: None,
        }
    }

    #[test]
    fn all_closed_work_is_release_ready() {
        let card = build_project_status(ProjectStatusInput {
            schema_version: PROJECT_STATUS_SCHEMA_VERSION,
            project: "CoreRoom".to_owned(),
            milestone: "v0.7".to_owned(),
            work_orders: vec![closed_work_order("WO-0217")],
            source_findings: Vec::new(),
            risks: Vec::new(),
        })
        .expect("card");

        assert!(card.release_ready);
        assert_eq!(card.decision, ReleaseDecision::ReadyToRelease);
        assert!(card.render_host_summary().contains("Release ready: true"));
    }
}
