//! Evidence Packet persistence and fixture coverage.

use std::fs;

use coreroom::config::COREROOM_DIR;
use coreroom::evidence_packet::{
    evidence_packet_path, load_evidence_packet, save_evidence_packet, ChangedFileEvidence,
    CommandEvidence, EvidencePacket, EvidenceResult, EvidenceStatus, RiskEvidence,
    RoleReviewEvidence, TestResultEvidence, TrackerUpdateEvidence, EVIDENCE_DIR,
};

#[test]
fn evidence_packet_roundtrips_v05_pr_example() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let packet = complete_packet();

    let path = save_evidence_packet(tmp.path(), &packet).expect("save");
    assert_eq!(
        path,
        tmp.path()
            .join(COREROOM_DIR)
            .join(EVIDENCE_DIR)
            .join("WO-0204.toml")
    );
    assert_eq!(
        path,
        evidence_packet_path(tmp.path(), "WO-0204").expect("path")
    );

    let content = fs::read_to_string(&path).expect("content");
    assert!(content.contains("workOrder = \"WO-0204\""));
    assert!(content.contains("pullRequest = 220"));
    assert!(content.contains("unverifiedItems"));

    let loaded = load_evidence_packet(&path).expect("load");
    assert_eq!(loaded, packet);
}

#[test]
fn incomplete_packet_summary_names_missing_evidence() {
    let mut packet = complete_packet();
    packet.status = EvidenceStatus::Incomplete;
    packet.changed_files.clear();
    packet.commands_run.clear();
    packet.test_results.clear();
    packet.role_reviews.clear();
    packet.tracker_update.evidence_ledger_updated = false;
    packet.unverified_items = vec!["CI status was not polled through GitHub API.".to_owned()];

    let report = packet.completion_report().expect("report");
    assert_eq!(report.status, EvidenceStatus::Incomplete);
    assert!(report.missing.contains(&"changed files".to_owned()));
    assert!(report.missing.contains(&"commands run".to_owned()));
    assert!(report
        .missing
        .contains(&"Evidence Ledger update".to_owned()));

    let summary = packet.render_pr_summary();
    assert!(summary.contains("Status: incomplete"));
    assert!(summary.contains("Missing:"));
    assert!(summary.contains("Not verified:"));
    assert!(summary.contains("CI status was not polled"));
}

#[test]
fn evidence_packet_fixtures_cover_complete_and_incomplete() {
    let complete = include_str!("fixtures/evidence_packet_v05_pr.toml");
    let incomplete = include_str!("fixtures/evidence_packet_incomplete.toml");

    let complete_packet: EvidencePacket = toml::from_str(complete).expect("complete fixture");
    let incomplete_packet: EvidencePacket = toml::from_str(incomplete).expect("incomplete fixture");

    assert_eq!(
        complete_packet
            .completion_report()
            .expect("complete report")
            .status,
        EvidenceStatus::Complete
    );
    assert_eq!(
        incomplete_packet
            .completion_report()
            .expect("incomplete report")
            .status,
        EvidenceStatus::Incomplete
    );
}

fn complete_packet() -> EvidencePacket {
    EvidencePacket {
        schema_version: coreroom::evidence_packet::EVIDENCE_PACKET_SCHEMA_VERSION,
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
            evidence: "rustfmt, shellcheck, clippy, macOS tests, Ubuntu tests passed".to_owned(),
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
        unverified_items: vec![
            "No CI API polling in v0.6; status copied from PR checks.".to_owned()
        ],
    }
}
