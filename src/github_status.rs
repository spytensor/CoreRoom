//! GitHub-native WorkOrder lifecycle facts.
//!
//! This module turns GitHub Issue, PR, CI, tracker, and Evidence Packet facts
//! into a structural lifecycle that `@host` can cite. It does not call GitHub.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::tracker::TrackerEntryState;

/// GitHub Issue state used for WorkOrder lifecycle derivation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum GitHubIssueState {
    /// Issue is not known or not yet created.
    Missing,
    /// Issue is open.
    Open,
    /// Issue is closed.
    Closed,
}

impl GitHubIssueState {
    /// Stable host-facing label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Open => "open",
            Self::Closed => "closed",
        }
    }
}

/// Pull request state.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PullRequestState {
    /// PR is open.
    Open,
    /// PR was closed without merge.
    Closed,
    /// PR was merged.
    Merged,
}

impl PullRequestState {
    /// Stable host-facing label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::Merged => "merged",
        }
    }
}

/// Pull request facts for a WorkOrder.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestFacts {
    /// GitHub PR number.
    pub number: u64,
    /// PR state.
    pub state: PullRequestState,
}

/// CI/check result state.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CheckState {
    /// Check is queued or running.
    Pending,
    /// Check passed.
    Pass,
    /// Check failed.
    Fail,
    /// Check was cancelled.
    Cancelled,
    /// Check was skipped and is not blocking.
    Skipped,
    /// Check state is unknown.
    Unknown,
}

impl CheckState {
    /// Stable host-facing label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Cancelled => "cancelled",
            Self::Skipped => "skipped",
            Self::Unknown => "unknown",
        }
    }

    const fn is_failure(self) -> bool {
        matches!(self, Self::Fail | Self::Cancelled)
    }
}

/// One GitHub check fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CheckFacts {
    /// Check name.
    pub name: String,
    /// Check state.
    pub state: CheckState,
    /// Optional URL for host citations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Local Evidence Packet state.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum EvidencePacketState {
    /// No packet is known.
    #[default]
    Missing,
    /// Packet exists but cannot support a completion claim.
    Incomplete,
    /// Packet is structurally complete.
    Complete,
}

impl EvidencePacketState {
    /// Stable host-facing label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Incomplete => "incomplete",
            Self::Complete => "complete",
        }
    }
}

/// GitHub and evidence facts for one WorkOrder.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitHubWorkOrderFacts {
    /// GitHub Issue number.
    pub issue: u64,
    /// GitHub Issue state.
    pub issue_state: GitHubIssueState,
    /// Labels currently attached to the issue.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// Implementation branch, if one exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Pull request facts, if one exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull_request: Option<PullRequestFacts>,
    /// GitHub check facts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<CheckFacts>,
    /// Tracker row state for this issue.
    #[serde(default)]
    pub tracker: TrackerEntryState,
    /// Evidence Packet state.
    #[serde(default)]
    pub evidence: EvidencePacketState,
    /// Evidence Packet id/path for citation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_packet: Option<String>,
    /// Optional explicit blocker reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
}

/// Canonical GitHub-native lifecycle for a WorkOrder.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum WorkOrderLifecycle {
    /// Issue exists or is desired, but no ready signal/branch/PR is present.
    NotStarted,
    /// Issue is ready to be picked up.
    Ready,
    /// Work has started on a branch, but no PR exists yet.
    InProgress,
    /// PR/review exists and completion is not yet final.
    InReview,
    /// CI/checks are failing or cancelled.
    FailedCi,
    /// Work cannot continue without human/input/rework.
    Blocked,
    /// PR is merged but evidence/tracker/closure is stale.
    MergedTrackerStale,
    /// Issue, PR, CI, evidence, and tracker are all closed.
    Closed,
}

impl WorkOrderLifecycle {
    /// Stable host-facing label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::NotStarted => "not-started",
            Self::Ready => "ready",
            Self::InProgress => "in-progress",
            Self::InReview => "in-review",
            Self::FailedCi => "failed-ci",
            Self::Blocked => "blocked",
            Self::MergedTrackerStale => "merged-tracker-stale",
            Self::Closed => "closed",
        }
    }
}

/// Derived lifecycle report for `@host`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitHubWorkOrderStatus {
    /// Derived lifecycle.
    pub lifecycle: WorkOrderLifecycle,
    /// Findings that prevent a completion claim or require host attention.
    pub findings: Vec<String>,
    /// Source facts that the host should cite.
    pub citations: Vec<String>,
}

impl GitHubWorkOrderStatus {
    /// Whether the WorkOrder is fully closed in GitHub/evidence/tracker terms.
    pub const fn is_closed(&self) -> bool {
        matches!(self.lifecycle, WorkOrderLifecycle::Closed)
    }

    /// Render a concise host-facing status summary.
    pub fn render_host_summary(&self, facts: &GitHubWorkOrderFacts) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "WorkOrder GitHub status for #{}", facts.issue);
        let _ = writeln!(out, "lifecycle: {}", self.lifecycle.label());
        let _ = writeln!(out, "issue: {}", facts.issue_state.label());
        let _ = writeln!(
            out,
            "branch: {}",
            facts.branch.as_deref().unwrap_or("unbound")
        );
        let _ = writeln!(
            out,
            "pullRequest: {}",
            facts.pull_request.as_ref().map_or_else(
                || "unbound".to_owned(),
                |pr| format!("#{} {}", pr.number, pr.state.label())
            )
        );
        let _ = writeln!(out, "checks: {}", format_checks(&facts.checks));
        let _ = writeln!(
            out,
            "tracker: checkbox={}, ledgerStatus={}, ledgerUpdated={}",
            facts.tracker.checkbox_checked,
            facts.tracker.ledger_status.as_deref().unwrap_or("unknown"),
            facts
                .tracker
                .ledger_tracker_updated
                .map_or_else(|| "unknown".to_owned(), |value| value.to_string())
        );
        let _ = writeln!(
            out,
            "evidence: {} {}",
            facts.evidence.label(),
            facts.evidence_packet.as_deref().unwrap_or("unbound")
        );
        write_list(&mut out, "citations", self.citations.iter().cloned());
        write_list(&mut out, "findings", self.findings.iter().cloned());
        out
    }
}

/// Derive a GitHub-native WorkOrder lifecycle from structural facts.
pub fn derive_github_work_order_status(facts: &GitHubWorkOrderFacts) -> GitHubWorkOrderStatus {
    let mut findings = findings(facts);
    let lifecycle = lifecycle(facts);
    if lifecycle == WorkOrderLifecycle::Closed {
        findings.clear();
    }
    GitHubWorkOrderStatus {
        lifecycle,
        findings,
        citations: citations(facts),
    }
}

fn lifecycle(facts: &GitHubWorkOrderFacts) -> WorkOrderLifecycle {
    if has_blocker(facts) {
        return WorkOrderLifecycle::Blocked;
    }
    if has_failed_check(facts) {
        return WorkOrderLifecycle::FailedCi;
    }
    if let Some(pr) = &facts.pull_request {
        return match pr.state {
            PullRequestState::Open => WorkOrderLifecycle::InReview,
            PullRequestState::Closed => WorkOrderLifecycle::Blocked,
            PullRequestState::Merged => {
                if facts.issue_state == GitHubIssueState::Closed
                    && facts.evidence == EvidencePacketState::Complete
                    && tracker_closed(&facts.tracker)
                {
                    WorkOrderLifecycle::Closed
                } else {
                    WorkOrderLifecycle::MergedTrackerStale
                }
            }
        };
    }
    if facts.branch.is_some() {
        return WorkOrderLifecycle::InProgress;
    }
    if facts.issue_state == GitHubIssueState::Open && has_ready_label(&facts.labels) {
        return WorkOrderLifecycle::Ready;
    }
    WorkOrderLifecycle::NotStarted
}

fn findings(facts: &GitHubWorkOrderFacts) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(blocker) = &facts.blocker {
        out.push(format!("blocked: {blocker}"));
    }
    if facts.labels.iter().any(|label| label == "status:blocked") {
        out.push("issue has status:blocked label".to_owned());
    }
    for check in facts.checks.iter().filter(|check| check.state.is_failure()) {
        out.push(format!(
            "CI check `{}` is {}",
            check.name,
            check.state.label()
        ));
    }
    if has_failed_check(facts) && facts.tracker.checkbox_checked {
        out.push("tracker checkbox is checked while CI is failing".to_owned());
    }
    if facts.issue_state == GitHubIssueState::Closed && !facts.tracker.checkbox_checked {
        out.push(format!(
            "issue #{} is closed but tracker checkbox is unchecked",
            facts.issue
        ));
    }
    if let Some(pr) = &facts.pull_request {
        if pr.state == PullRequestState::Closed {
            out.push(format!("PR #{} is closed without merge", pr.number));
        }
        if pr.state == PullRequestState::Merged {
            if facts.evidence != EvidencePacketState::Complete {
                out.push(format!(
                    "PR #{} is merged but Evidence Packet is {}",
                    pr.number,
                    facts.evidence.label()
                ));
            }
            if !tracker_closed(&facts.tracker) {
                out.push(format!(
                    "PR #{} is merged but tracker row is not merged/yes",
                    pr.number
                ));
            }
            if facts.issue_state != GitHubIssueState::Closed {
                out.push(format!(
                    "PR #{} is merged but linked issue #{} is not closed",
                    pr.number, facts.issue
                ));
            }
        }
    }
    out
}

fn citations(facts: &GitHubWorkOrderFacts) -> Vec<String> {
    let mut out = vec![format!(
        "issue:#{}:{}",
        facts.issue,
        facts.issue_state.label()
    )];
    if let Some(branch) = &facts.branch {
        out.push(format!("branch:{branch}"));
    }
    if let Some(pr) = &facts.pull_request {
        out.push(format!("pr:#{}:{}", pr.number, pr.state.label()));
    }
    if !facts.checks.is_empty() {
        out.push(format!("checks:{}", format_checks(&facts.checks)));
    }
    out.push(format!(
        "tracker:checkbox={},ledger={},updated={}",
        facts.tracker.checkbox_checked,
        facts.tracker.ledger_status.as_deref().unwrap_or("unknown"),
        facts
            .tracker
            .ledger_tracker_updated
            .map_or_else(|| "unknown".to_owned(), |value| value.to_string())
    ));
    out.push(format!(
        "evidence:{}:{}",
        facts.evidence.label(),
        facts.evidence_packet.as_deref().unwrap_or("unbound")
    ));
    out
}

fn tracker_closed(tracker: &TrackerEntryState) -> bool {
    tracker.checkbox_checked
        && tracker.ledger_status.as_deref() == Some("merged")
        && tracker.ledger_tracker_updated == Some(true)
}

fn has_ready_label(labels: &[String]) -> bool {
    labels
        .iter()
        .any(|label| matches!(label.as_str(), "status:ready" | "codex-ready"))
}

fn has_blocker(facts: &GitHubWorkOrderFacts) -> bool {
    facts.blocker.is_some() || facts.labels.iter().any(|label| label == "status:blocked")
}

fn has_failed_check(facts: &GitHubWorkOrderFacts) -> bool {
    facts.checks.iter().any(|check| check.state.is_failure())
}

fn format_checks(checks: &[CheckFacts]) -> String {
    if checks.is_empty() {
        return "none".to_owned();
    }
    checks
        .iter()
        .map(|check| format!("{}={}", check.name, check.state.label()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn write_list<I>(out: &mut String, title: &str, items: I)
where
    I: IntoIterator<Item = String>,
{
    let items = items.into_iter().collect::<Vec<_>>();
    if items.is_empty() {
        let _ = writeln!(out, "{title}: none");
    } else {
        let _ = writeln!(out, "{title}:");
        for item in items {
            let _ = writeln!(out, "- {item}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> GitHubWorkOrderFacts {
        GitHubWorkOrderFacts {
            issue: 214,
            issue_state: GitHubIssueState::Open,
            labels: vec!["status:ready".to_owned(), "codex-ready".to_owned()],
            branch: None,
            pull_request: None,
            checks: Vec::new(),
            tracker: TrackerEntryState::default(),
            evidence: EvidencePacketState::Missing,
            evidence_packet: None,
            blocker: None,
        }
    }

    #[test]
    fn ready_issue_without_branch_is_ready() {
        let report = derive_github_work_order_status(&facts());

        assert_eq!(report.lifecycle, WorkOrderLifecycle::Ready);
        assert!(report.findings.is_empty());
    }

    #[test]
    fn merged_pr_requires_evidence_and_tracker_before_closed() {
        let mut facts = facts();
        facts.issue_state = GitHubIssueState::Closed;
        facts.pull_request = Some(PullRequestFacts {
            number: 230,
            state: PullRequestState::Merged,
        });
        facts.checks = vec![CheckFacts {
            name: "clippy".to_owned(),
            state: CheckState::Pass,
            url: None,
        }];
        facts.evidence = EvidencePacketState::Incomplete;

        let report = derive_github_work_order_status(&facts);

        assert_eq!(report.lifecycle, WorkOrderLifecycle::MergedTrackerStale);
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.contains("Evidence Packet")));
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.contains("tracker row")));
    }

    #[test]
    fn failed_ci_wins_even_if_tracker_is_checked() {
        let mut facts = facts();
        facts.pull_request = Some(PullRequestFacts {
            number: 230,
            state: PullRequestState::Open,
        });
        facts.checks = vec![CheckFacts {
            name: "test (ubuntu-latest)".to_owned(),
            state: CheckState::Fail,
            url: None,
        }];
        facts.tracker.checkbox_checked = true;
        facts.tracker.ledger_status = Some("merged".to_owned());
        facts.tracker.ledger_tracker_updated = Some(true);

        let report = derive_github_work_order_status(&facts);

        assert_eq!(report.lifecycle, WorkOrderLifecycle::FailedCi);
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.contains("CI check")));
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.contains("tracker checkbox")));
    }

    #[test]
    fn fully_closed_requires_issue_pr_evidence_and_tracker() {
        let mut facts = facts();
        facts.issue_state = GitHubIssueState::Closed;
        facts.pull_request = Some(PullRequestFacts {
            number: 230,
            state: PullRequestState::Merged,
        });
        facts.checks = vec![CheckFacts {
            name: "rustfmt".to_owned(),
            state: CheckState::Pass,
            url: None,
        }];
        facts.tracker.checkbox_checked = true;
        facts.tracker.ledger_status = Some("merged".to_owned());
        facts.tracker.ledger_tracker_updated = Some(true);
        facts.evidence = EvidencePacketState::Complete;
        facts.evidence_packet = Some("WO-0214".to_owned());

        let report = derive_github_work_order_status(&facts);

        assert_eq!(report.lifecycle, WorkOrderLifecycle::Closed);
        assert!(report.is_closed());
        assert!(report.findings.is_empty());
    }
}
