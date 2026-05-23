//! Evidence Packet model for host-led completion claims.
//!
//! Evidence Packets tie a WorkOrder to structured proof: issue, branch, PR,
//! gate thread, changed files, commands, tests, reviews, risks, rollback, and
//! tracker state. Model prose can summarize this evidence, but it is not
//! evidence by itself.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::COREROOM_DIR;

/// Subdirectory inside `.coreroom/` that stores Evidence Packets.
pub const EVIDENCE_DIR: &str = "evidence";

/// Current persisted Evidence Packet schema version.
pub const EVIDENCE_PACKET_SCHEMA_VERSION: u32 = 1;

/// Evidence Packet status.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceStatus {
    /// Packet is not complete enough to support a completion claim.
    #[default]
    Incomplete,
    /// Packet is complete enough for PR-ready summary.
    Complete,
}

impl EvidenceStatus {
    /// Stable label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Incomplete => "incomplete",
            Self::Complete => "complete",
        }
    }
}

/// Command or test status.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceResult {
    /// Evidence passed.
    Pass,
    /// Evidence failed.
    Fail,
    /// Evidence was not run or not available.
    NotRun,
}

impl EvidenceResult {
    /// Stable label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::NotRun => "not-run",
        }
    }
}

/// Changed file evidence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChangedFileEvidence {
    /// Repository-relative file path.
    pub path: String,
    /// Short change summary.
    pub summary: String,
}

/// Command execution evidence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommandEvidence {
    /// Command that was run.
    pub command: String,
    /// Command result.
    pub result: EvidenceResult,
    /// Output summary or cited log reference.
    pub evidence: String,
}

/// Test result evidence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TestResultEvidence {
    /// Test suite or check name.
    pub name: String,
    /// Test result.
    pub result: EvidenceResult,
    /// Output summary or cited log reference.
    pub evidence: String,
}

/// Role review evidence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RoleReviewEvidence {
    /// Reviewer role.
    pub role: String,
    /// Review decision or status.
    pub decision: String,
    /// Review evidence path, PR comment, or summary.
    pub evidence: String,
}

/// Risk evidence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RiskEvidence {
    /// Risk level such as `none`, `low`, `medium`, or `high`.
    pub level: String,
    /// Risk description.
    pub description: String,
}

/// Tracker update evidence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct TrackerUpdateEvidence {
    /// Tracker issue number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracker_issue: Option<u64>,
    /// Whether the issue checkbox was updated.
    pub checkbox_updated: bool,
    /// Whether the Evidence Ledger row was updated.
    pub evidence_ledger_updated: bool,
    /// Milestone acceptance criteria updated by this packet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub milestone_ac_updated: Vec<String>,
}

/// Persisted Evidence Packet.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvidencePacket {
    /// Schema version.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Packet status.
    #[serde(default)]
    pub status: EvidenceStatus,
    /// Bound WorkOrder id.
    pub work_order: String,
    /// Bound GitHub Issue number.
    pub github_issue: u64,
    /// Branch that carried the work.
    pub branch: String,
    /// Pull request number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull_request: Option<u64>,
    /// SDLC gate thread id.
    pub gate_thread: String,
    /// Changed files.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed_files: Vec<ChangedFileEvidence>,
    /// Commands run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands_run: Vec<CommandEvidence>,
    /// Test/check results.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub test_results: Vec<TestResultEvidence>,
    /// Role reviews.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub role_reviews: Vec<RoleReviewEvidence>,
    /// Explicit risks or `none` statement.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub risks: Vec<RiskEvidence>,
    /// Rollback plan.
    pub rollback: String,
    /// Tracker update status.
    #[serde(default)]
    pub tracker_update: TrackerUpdateEvidence,
    /// Explicit list of items not verified.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unverified_items: Vec<String>,
}

impl EvidencePacket {
    /// Validate shape and return missing evidence for completion claims.
    pub fn completion_report(&self) -> Result<EvidenceCompletionReport> {
        self.validate_shape()?;
        let mut missing = Vec::new();

        require_nonempty_collection(&self.changed_files, "changed files", &mut missing);
        require_nonempty_collection(&self.commands_run, "commands run", &mut missing);
        require_nonempty_collection(&self.test_results, "test results", &mut missing);
        require_nonempty_collection(&self.role_reviews, "role reviews", &mut missing);
        require_nonempty_collection(&self.risks, "risk statement", &mut missing);
        if self.rollback.trim().is_empty() {
            missing.push("rollback plan".to_owned());
        }
        if !self.tracker_update.checkbox_updated {
            missing.push("tracker checkbox update".to_owned());
        }
        if !self.tracker_update.evidence_ledger_updated {
            missing.push("Evidence Ledger update".to_owned());
        }
        if self.pull_request.is_none() {
            missing.push("pull request".to_owned());
        }
        if !self
            .commands_run
            .iter()
            .any(|command| command.result == EvidenceResult::Pass)
            && !self
                .test_results
                .iter()
                .any(|test| test.result == EvidenceResult::Pass)
        {
            missing.push("passing command or test evidence".to_owned());
        }
        if missing.is_empty() && self.status != EvidenceStatus::Complete {
            missing.push("packet status marked complete".to_owned());
        }

        Ok(EvidenceCompletionReport {
            status: if missing.is_empty() {
                EvidenceStatus::Complete
            } else {
                EvidenceStatus::Incomplete
            },
            missing,
            unverified_items: self.unverified_items.clone(),
        })
    }

    /// Render a PR-ready evidence summary from structured fields.
    pub fn render_pr_summary(&self) -> String {
        let report = self
            .completion_report()
            .unwrap_or_else(|err| EvidenceCompletionReport {
                status: EvidenceStatus::Incomplete,
                missing: vec![err.to_string()],
                unverified_items: Vec::new(),
            });

        let mut out = String::new();
        let _ = writeln!(out, "## Evidence Packet");
        let _ = writeln!(out);
        let _ = writeln!(out, "- WorkOrder: {}", self.work_order);
        let _ = writeln!(out, "- GitHub Issue: #{}", self.github_issue);
        let _ = writeln!(out, "- Branch: `{}`", self.branch);
        let _ = writeln!(
            out,
            "- PR: {}",
            self.pull_request.map_or_else(
                || "unbound".to_owned(),
                |pull_request| format!("#{pull_request}")
            )
        );
        let _ = writeln!(out, "- Gate thread: `{}`", self.gate_thread);
        let _ = writeln!(out, "- Status: {}", report.status.label());
        write_list(
            &mut out,
            "Changed files",
            self.changed_files
                .iter()
                .map(|file| format!("`{}` - {}", file.path, file.summary)),
        );
        write_list(
            &mut out,
            "Commands",
            self.commands_run.iter().map(|command| {
                format!(
                    "`{}` - {} - {}",
                    command.command,
                    command.result.label(),
                    command.evidence
                )
            }),
        );
        write_list(
            &mut out,
            "Tests",
            self.test_results.iter().map(|test| {
                format!(
                    "{} - {} - {}",
                    test.name,
                    test.result.label(),
                    test.evidence
                )
            }),
        );
        write_list(
            &mut out,
            "Role reviews",
            self.role_reviews.iter().map(|review| {
                format!(
                    "@{} - {} - {}",
                    review.role, review.decision, review.evidence
                )
            }),
        );
        write_list(
            &mut out,
            "Risks",
            self.risks
                .iter()
                .map(|risk| format!("{} - {}", risk.level, risk.description)),
        );
        let _ = writeln!(out, "Rollback: {}", self.rollback);
        let _ = writeln!(
            out,
            "Tracker: checkbox={}, evidenceLedger={}, issue={}",
            self.tracker_update.checkbox_updated,
            self.tracker_update.evidence_ledger_updated,
            self.tracker_update
                .tracker_issue
                .map_or_else(|| "unbound".to_owned(), |issue| format!("#{issue}"))
        );
        write_list(&mut out, "Missing", report.missing.iter().cloned());
        write_list(
            &mut out,
            "Not verified",
            report.unverified_items.iter().cloned(),
        );
        out
    }

    fn validate_shape(&self) -> Result<()> {
        if self.schema_version != EVIDENCE_PACKET_SCHEMA_VERSION {
            bail!(
                "unsupported Evidence Packet schemaVersion {}; expected {}",
                self.schema_version,
                EVIDENCE_PACKET_SCHEMA_VERSION
            );
        }
        validate_work_order_id(&self.work_order)?;
        ensure_positive("githubIssue", self.github_issue)?;
        ensure_positive_option("pullRequest", self.pull_request)?;
        ensure_nonempty("branch", &self.branch)?;
        ensure_nonempty("gateThread", &self.gate_thread)?;
        ensure_nonempty("rollback", &self.rollback)?;
        ensure_positive_option("trackerIssue", self.tracker_update.tracker_issue)?;
        for file in &self.changed_files {
            ensure_nonempty("changedFiles.path", &file.path)?;
            ensure_nonempty("changedFiles.summary", &file.summary)?;
        }
        for command in &self.commands_run {
            ensure_nonempty("commandsRun.command", &command.command)?;
            ensure_nonempty("commandsRun.evidence", &command.evidence)?;
        }
        for test in &self.test_results {
            ensure_nonempty("testResults.name", &test.name)?;
            ensure_nonempty("testResults.evidence", &test.evidence)?;
        }
        for review in &self.role_reviews {
            ensure_nonempty("roleReviews.role", &review.role)?;
            ensure_nonempty("roleReviews.decision", &review.decision)?;
            ensure_nonempty("roleReviews.evidence", &review.evidence)?;
        }
        for risk in &self.risks {
            ensure_nonempty("risks.level", &risk.level)?;
            ensure_nonempty("risks.description", &risk.description)?;
        }
        Ok(())
    }
}

/// Completion status report for an Evidence Packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceCompletionReport {
    /// Complete or incomplete.
    pub status: EvidenceStatus,
    /// Missing structural evidence.
    pub missing: Vec<String>,
    /// Explicitly unverified items.
    pub unverified_items: Vec<String>,
}

/// Save an Evidence Packet to `.coreroom/evidence/<workOrder>.toml`.
pub fn save_evidence_packet(project_root: &Path, packet: &EvidencePacket) -> Result<PathBuf> {
    packet.completion_report()?;
    let path = evidence_packet_path(project_root, &packet.work_order)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let content = toml::to_string_pretty(packet).context("serializing Evidence Packet")?;
    std::fs::write(&path, ensure_trailing_newline(&content))
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

/// Load an Evidence Packet from TOML.
pub fn load_evidence_packet(path: &Path) -> Result<EvidencePacket> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let packet: EvidencePacket =
        toml::from_str(&content).with_context(|| format!("parsing {}", path.display()))?;
    packet.completion_report()?;
    Ok(packet)
}

/// Return the canonical packet path for a WorkOrder.
pub fn evidence_packet_path(project_root: &Path, work_order: &str) -> Result<PathBuf> {
    validate_work_order_id(work_order)?;
    Ok(project_root
        .join(COREROOM_DIR)
        .join(EVIDENCE_DIR)
        .join(format!("{work_order}.toml")))
}

fn default_schema_version() -> u32 {
    EVIDENCE_PACKET_SCHEMA_VERSION
}

fn require_nonempty_collection<T>(items: &[T], label: &str, missing: &mut Vec<String>) {
    if items.is_empty() {
        missing.push(label.to_owned());
    }
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

fn validate_work_order_id(id: &str) -> Result<()> {
    let Some(number) = id.strip_prefix("WO-") else {
        bail!("workOrder `{id}` must start with `WO-`");
    };
    if number.is_empty() || !number.chars().all(|ch| ch.is_ascii_digit()) {
        bail!("workOrder `{id}` must use `WO-<digits>`");
    }
    Ok(())
}

fn ensure_nonempty(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{field} cannot be empty");
    }
    Ok(())
}

fn ensure_positive(field: &str, value: u64) -> Result<()> {
    if value == 0 {
        bail!("{field} must be greater than zero");
    }
    Ok(())
}

fn ensure_positive_option(field: &str, value: Option<u64>) -> Result<()> {
    if value == Some(0) {
        bail!("{field} must be greater than zero");
    }
    Ok(())
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

    fn complete_packet() -> EvidencePacket {
        EvidencePacket {
            schema_version: EVIDENCE_PACKET_SCHEMA_VERSION,
            status: EvidenceStatus::Complete,
            work_order: "WO-0204".to_owned(),
            github_issue: 204,
            branch: "feat/v0.6-204-host-authority-protocol".to_owned(),
            pull_request: Some(220),
            gate_thread: "thread-host-protocol".to_owned(),
            changed_files: vec![ChangedFileEvidence {
                path: "docs/sdlc-gates.md".to_owned(),
                summary: "Document host authority protocol.".to_owned(),
            }],
            commands_run: vec![CommandEvidence {
                command: "cargo test role::tests --quiet".to_owned(),
                result: EvidenceResult::Pass,
                evidence: "role tests passed".to_owned(),
            }],
            test_results: vec![TestResultEvidence {
                name: "GitHub CI".to_owned(),
                result: EvidenceResult::Pass,
                evidence: "rustfmt, shellcheck, clippy, macOS tests, Ubuntu tests passed"
                    .to_owned(),
            }],
            role_reviews: vec![RoleReviewEvidence {
                role: "host".to_owned(),
                decision: "accepted".to_owned(),
                evidence: "A-017 accepted by user".to_owned(),
            }],
            risks: vec![RiskEvidence {
                level: "low".to_owned(),
                description: "Docs and priors only; no state migration.".to_owned(),
            }],
            rollback: "Revert PR #220; no migration included.".to_owned(),
            tracker_update: TrackerUpdateEvidence {
                tracker_issue: Some(202),
                checkbox_updated: true,
                evidence_ledger_updated: true,
                milestone_ac_updated: vec!["M-AC-2".to_owned()],
            },
            unverified_items: Vec::new(),
        }
    }

    #[test]
    fn complete_packet_reports_complete_and_renders_summary() {
        let packet = complete_packet();

        let report = packet.completion_report().expect("report");
        assert_eq!(report.status, EvidenceStatus::Complete);
        assert!(report.missing.is_empty());

        let summary = packet.render_pr_summary();
        assert!(summary.contains("WorkOrder: WO-0204"));
        assert!(summary.contains("GitHub Issue: #204"));
        assert!(summary.contains("Status: complete"));
        assert!(summary.contains("Changed files:"));
    }

    #[test]
    fn incomplete_packet_lists_missing_evidence() {
        let mut packet = complete_packet();
        packet.status = EvidenceStatus::Incomplete;
        packet.commands_run.clear();
        packet.test_results.clear();
        packet.tracker_update.checkbox_updated = false;
        packet.unverified_items = vec!["No CI API polling in v0.6.".to_owned()];

        let report = packet.completion_report().expect("report");
        assert_eq!(report.status, EvidenceStatus::Incomplete);
        assert!(report.missing.contains(&"commands run".to_owned()));
        assert!(report.missing.contains(&"test results".to_owned()));
        assert!(report
            .missing
            .contains(&"passing command or test evidence".to_owned()));
        assert_eq!(report.unverified_items, ["No CI API polling in v0.6."]);
    }
}
