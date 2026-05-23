//! Tracker issue parsing and stale-completion detection.
//!
//! This is a structural helper for `@host`: it detects when implementation,
//! PR, evidence, and tracker state disagree. It does not call GitHub APIs.

use serde::{Deserialize, Serialize};

/// External facts about an issue/work item.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrackerWorkState {
    /// GitHub Issue number.
    pub issue: u64,
    /// Whether the issue is closed.
    pub issue_closed: bool,
    /// Whether the implementation PR has merged.
    pub pr_merged: bool,
    /// Whether a local Evidence Packet exists.
    pub evidence_packet_exists: bool,
}

/// Parsed tracker state for one issue row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TrackerEntryState {
    /// Whether the tracker checkbox is ticked.
    pub checkbox_checked: bool,
    /// Evidence Ledger status cell, such as `pending`, `in_review`, or `merged`.
    pub ledger_status: Option<String>,
    /// Whether the Evidence Ledger tracker-updated cell says `yes`.
    pub ledger_tracker_updated: Option<bool>,
}

/// Mismatch report for tracker closure.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TrackerMismatchReport {
    /// Parsed tracker state for the issue.
    pub entry: TrackerEntryState,
    /// Human-readable mismatch findings.
    pub findings: Vec<String>,
}

impl TrackerMismatchReport {
    /// Whether no stale tracker findings were detected.
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }

    /// Host-facing tracker patch summary.
    pub fn propose_tracker_patch(&self, state: &TrackerWorkState) -> String {
        if self.is_clean() {
            return format!("#{} tracker state is consistent.", state.issue);
        }
        let mut lines = vec![format!(
            "Tracker patch needed for #{} before claiming done:",
            state.issue
        )];
        if state.issue_closed && !self.entry.checkbox_checked {
            lines.push("- Tick the issue checkbox in the tracker.".to_owned());
        }
        if state.pr_merged
            && (self.entry.ledger_status.as_deref() != Some("merged")
                || self.entry.ledger_tracker_updated != Some(true))
        {
            lines.push(
                "- Update the Evidence Ledger row to merged and Tracker Updated = yes.".to_owned(),
            );
        }
        if state.evidence_packet_exists && !state.issue_closed {
            lines.push(
                "- Close the linked issue or explain why evidence is not completion.".to_owned(),
            );
        }
        lines.join("\n")
    }
}

/// Detect stale tracker state from a tracker issue body and external facts.
pub fn detect_tracker_mismatch(
    tracker_body: &str,
    state: &TrackerWorkState,
) -> TrackerMismatchReport {
    let entry = parse_tracker_entry(tracker_body, state.issue);
    let mut findings = Vec::new();

    if state.issue_closed && !entry.checkbox_checked {
        findings.push(format!(
            "issue #{} is closed but tracker checkbox is unchecked",
            state.issue
        ));
    }
    if state.pr_merged
        && (entry.ledger_status.as_deref() != Some("merged")
            || entry.ledger_tracker_updated != Some(true))
    {
        findings.push(format!(
            "issue #{} PR is merged but Evidence Ledger is not merged/yes",
            state.issue
        ));
    }
    if state.evidence_packet_exists && !state.issue_closed {
        findings.push(format!(
            "issue #{} has evidence but linked issue is not closed",
            state.issue
        ));
    }

    TrackerMismatchReport { entry, findings }
}

fn parse_tracker_entry(tracker_body: &str, issue: u64) -> TrackerEntryState {
    let checkbox_checked = tracker_body.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.starts_with(&format!("- [x] #{issue} "))
            || trimmed.starts_with(&format!("- [X] #{issue} "))
    });

    let mut ledger_status = None;
    let mut ledger_tracker_updated = None;
    for line in tracker_body.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with(&format!("| #{issue} |")) {
            continue;
        }
        let cells = trimmed
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect::<Vec<_>>();
        if cells.len() >= 5 {
            ledger_status = Some(cells[2].to_owned());
            ledger_tracker_updated = match cells[4].to_ascii_lowercase().as_str() {
                "yes" | "true" => Some(true),
                "no" | "false" => Some(false),
                _ => None,
            };
        }
    }

    TrackerEntryState {
        checkbox_checked,
        ledger_status,
        ledger_tracker_updated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRACKER: &str = r"
## Phase 4

- [ ] #211 - Mandatory tracker update protocol
- [x] #210 - Evidence Packet model

| Issue | PR | Status | Evidence | Tracker Updated |
|---|---:|---|---|---|
| #210 | #226 | merged | CI passed | yes |
| #211 | #227 | pending | TBD | no |
";

    #[test]
    fn detects_closed_issue_with_unchecked_tracker() {
        let report = detect_tracker_mismatch(
            TRACKER,
            &TrackerWorkState {
                issue: 211,
                issue_closed: true,
                pr_merged: false,
                evidence_packet_exists: false,
            },
        );

        assert!(!report.is_clean());
        assert!(report.findings[0].contains("closed but tracker checkbox"));
        assert!(report
            .propose_tracker_patch(&TrackerWorkState {
                issue: 211,
                issue_closed: true,
                pr_merged: false,
                evidence_packet_exists: false,
            })
            .contains("Tick the issue checkbox"));
    }

    #[test]
    fn detects_merged_pr_with_pending_evidence_ledger() {
        let report = detect_tracker_mismatch(
            TRACKER,
            &TrackerWorkState {
                issue: 211,
                issue_closed: false,
                pr_merged: true,
                evidence_packet_exists: false,
            },
        );

        assert!(!report.is_clean());
        assert!(report.findings[0].contains("Evidence Ledger"));
    }

    #[test]
    fn detects_evidence_without_issue_closure() {
        let report = detect_tracker_mismatch(
            TRACKER,
            &TrackerWorkState {
                issue: 211,
                issue_closed: false,
                pr_merged: false,
                evidence_packet_exists: true,
            },
        );

        assert!(!report.is_clean());
        assert!(report.findings[0].contains("has evidence"));
    }

    #[test]
    fn clean_when_tracker_and_external_state_agree() {
        let report = detect_tracker_mismatch(
            TRACKER,
            &TrackerWorkState {
                issue: 210,
                issue_closed: true,
                pr_merged: true,
                evidence_packet_exists: true,
            },
        );

        assert!(report.is_clean());
    }
}
