//! Console projection fixtures.

use coreroom::console_projection::{
    project_evidence_snapshot, project_gate_snapshot, project_source_health_snapshot,
    project_work_snapshot, GateProjectionInput,
};
use coreroom::console_snapshot::{
    EvidenceClosureState, SourceHealthState, StatusState, WorkLifecycle,
};
use coreroom::evidence_packet::{EvidencePacket, EvidenceStatus};
use coreroom::github_status::{
    CheckFacts, CheckState, EvidencePacketState, GitHubIssueState, GitHubWorkOrderFacts,
    PullRequestFacts, PullRequestState,
};
use coreroom::source_graph::{SourceGraphFinding, SourceGraphFindingKind};
use coreroom::source_registry::SourceTrustLevel;
use coreroom::tracker::TrackerEntryState;
use coreroom::work_order::{RequiredEvidence, WorkOrder, WorkOrderStatus};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectionFixture {
    cases: Vec<ProjectionCase>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectionCase {
    name: String,
    expected_lifecycle: WorkLifecycle,
    expected_ci: StatusState,
    expected_evidence: StatusState,
    expected_tracker: StatusState,
}

#[test]
fn work_projection_preserves_status_fields_across_fixture_cases() {
    let fixture: ProjectionFixture =
        toml::from_str(include_str!("fixtures/console_projection_cases.toml")).expect("fixture");

    for case in fixture.cases {
        let work = work_order(&case.name);
        let row = project_work_snapshot(
            &work,
            &github_facts(&case.name),
            vec!["source:core-repo@commit:fixture".to_owned()],
        );
        assert_eq!(row.lifecycle, case.expected_lifecycle, "{}", case.name);
        assert_eq!(row.ci_state, case.expected_ci, "{}", case.name);
        assert_eq!(row.evidence_state, case.expected_evidence, "{}", case.name);
        assert_eq!(row.tracker_state, case.expected_tracker, "{}", case.name);
        assert_eq!(row.phase.as_deref(), Some("v0.8"));
        assert_eq!(row.epic.as_deref(), Some("console-projections"));
        assert!(!row.source_citations.is_empty());
    }
}

#[test]
fn gate_projection_preserves_blockers_reviews_plan_sha_and_signoff() {
    let gate = project_gate_snapshot(GateProjectionInput {
        work_order: "WO-0247".to_owned(),
        current_phase: "review".to_owned(),
        blocked_reason: Some("security veto".to_owned()),
        missing_reviews: vec!["qa".to_owned(), "security".to_owned()],
        stale_plan_sha: Some("sha256:old-plan".to_owned()),
        signoff_ready: false,
    });

    assert_eq!(gate.work_order, "WO-0247");
    assert_eq!(gate.blocked_reason.as_deref(), Some("security veto"));
    assert_eq!(gate.missing_reviews.len(), 2);
    assert_eq!(gate.stale_plan_sha.as_deref(), Some("sha256:old-plan"));
    assert!(!gate.signoff_ready);
}

#[test]
fn evidence_projection_covers_complete_incomplete_missing_and_unverified() {
    let complete = project_evidence_snapshot("WO-0247", Some(&evidence_packet(true)));
    assert_eq!(complete.status, EvidenceClosureState::Complete);
    assert!(complete.tracker_updated);

    let incomplete = project_evidence_snapshot("WO-0247", Some(&evidence_packet(false)));
    assert_eq!(incomplete.status, EvidenceClosureState::Incomplete);
    assert!(!incomplete.missing_fields.is_empty());

    let missing = project_evidence_snapshot("WO-0247", None);
    assert_eq!(missing.status, EvidenceClosureState::Missing);
    assert_eq!(missing.missing_fields, vec!["Evidence Packet"]);
}

#[test]
fn source_projection_covers_drift_missing_trust_and_visibility() {
    for (kind, expected) in [
        (
            SourceGraphFindingKind::FileHashChanged,
            SourceHealthState::Stale,
        ),
        (
            SourceGraphFindingKind::MissingSource,
            SourceHealthState::Missing,
        ),
        (
            SourceGraphFindingKind::TrustChanged,
            SourceHealthState::TrustChanged,
        ),
        (
            SourceGraphFindingKind::VisibilityDenied,
            SourceHealthState::VisibilityDenied,
        ),
    ] {
        let row = project_source_health_snapshot(
            &SourceGraphFinding::new("core-api", kind, format!("finding {}", kind.label())),
            SourceTrustLevel::Internal,
            vec!["backend".to_owned()],
            vec!["WO-0247".to_owned()],
        );
        assert_eq!(row.status, expected);
        assert_eq!(row.trust_level, "internal");
        assert_eq!(row.related_work_orders, vec!["WO-0247"]);
    }
}

fn work_order(name: &str) -> WorkOrder {
    WorkOrder {
        schema_version: coreroom::work_order::WORK_ORDER_SCHEMA_VERSION,
        id: "WO-0247".to_owned(),
        title: format!("{name} projection"),
        objective: "Project console facts without losing evidence.".to_owned(),
        github_issue: Some(247),
        phase: Some("v0.8".to_owned()),
        epic: Some("console-projections".to_owned()),
        gate_thread: Some("thread-247".to_owned()),
        branch: Some("feat/v0.8-247-console-projections".to_owned()),
        pull_request: None,
        status: WorkOrderStatus::InReview,
        acceptance_criteria: vec!["project facts".to_owned()],
        required_evidence: vec![RequiredEvidence::Validation],
        tracker_issue: Some(238),
        tracker_checkbox: Some("#247".to_owned()),
    }
}

fn github_facts(name: &str) -> GitHubWorkOrderFacts {
    let mut facts = GitHubWorkOrderFacts {
        issue: 247,
        issue_state: GitHubIssueState::Open,
        labels: vec!["status:ready".to_owned()],
        branch: Some("feat/v0.8-247-console-projections".to_owned()),
        pull_request: None,
        checks: Vec::new(),
        tracker: TrackerEntryState::default(),
        evidence: EvidencePacketState::Incomplete,
        evidence_packet: Some("WO-0247".to_owned()),
        blocker: None,
    };
    match name {
        "healthy" => {
            facts.issue_state = GitHubIssueState::Closed;
            facts.pull_request = Some(PullRequestFacts {
                number: 276,
                state: PullRequestState::Merged,
            });
            facts.checks = vec![check("clippy", CheckState::Pass)];
            facts.evidence = EvidencePacketState::Complete;
            facts.tracker.checkbox_checked = true;
            facts.tracker.ledger_status = Some("merged".to_owned());
            facts.tracker.ledger_tracker_updated = Some(true);
        }
        "blocked" => {
            facts.blocker = Some("waiting for user decision".to_owned());
        }
        "failed-ci" => {
            facts.pull_request = Some(PullRequestFacts {
                number: 276,
                state: PullRequestState::Open,
            });
            facts.checks = vec![check("test (ubuntu-latest)", CheckState::Fail)];
        }
        "merged-tracker-stale" => {
            facts.issue_state = GitHubIssueState::Closed;
            facts.pull_request = Some(PullRequestFacts {
                number: 276,
                state: PullRequestState::Merged,
            });
            facts.checks = vec![check("clippy", CheckState::Pass)];
            facts.evidence = EvidencePacketState::Complete;
            facts.tracker.checkbox_checked = false;
        }
        _ => {}
    }
    facts
}

fn check(name: &str, state: CheckState) -> CheckFacts {
    CheckFacts {
        name: name.to_owned(),
        state,
        url: Some(format!("https://github.example/checks/{name}")),
    }
}

fn evidence_packet(complete: bool) -> EvidencePacket {
    let mut packet = EvidencePacket {
        schema_version: coreroom::evidence_packet::EVIDENCE_PACKET_SCHEMA_VERSION,
        status: if complete {
            EvidenceStatus::Complete
        } else {
            EvidenceStatus::Incomplete
        },
        work_order: "WO-0247".to_owned(),
        github_issue: 247,
        branch: "feat/v0.8-247-console-projections".to_owned(),
        pull_request: Some(276),
        gate_thread: "thread-247".to_owned(),
        changed_files: Vec::new(),
        commands_run: Vec::new(),
        test_results: Vec::new(),
        role_reviews: Vec::new(),
        risks: Vec::new(),
        rollback: "Revert PR #276.".to_owned(),
        tracker_update: coreroom::evidence_packet::TrackerUpdateEvidence {
            tracker_issue: Some(238),
            checkbox_updated: complete,
            evidence_ledger_updated: complete,
            milestone_ac_updated: Vec::new(),
        },
        unverified_items: Vec::new(),
    };
    if complete {
        packet
            .changed_files
            .push(coreroom::evidence_packet::ChangedFileEvidence {
                path: "src/console_projection.rs".to_owned(),
                summary: "Projection helpers.".to_owned(),
            });
        packet
            .commands_run
            .push(coreroom::evidence_packet::CommandEvidence {
                command: "cargo test --test console_projection_test --quiet".to_owned(),
                result: coreroom::evidence_packet::EvidenceResult::Pass,
                evidence: "projection tests passed".to_owned(),
            });
        packet
            .test_results
            .push(coreroom::evidence_packet::TestResultEvidence {
                name: "projection tests".to_owned(),
                result: coreroom::evidence_packet::EvidenceResult::Pass,
                evidence: "passed".to_owned(),
            });
        packet
            .role_reviews
            .push(coreroom::evidence_packet::RoleReviewEvidence {
                role: "reviewer".to_owned(),
                decision: "accepted".to_owned(),
                evidence: "fixture".to_owned(),
            });
        packet.risks.push(coreroom::evidence_packet::RiskEvidence {
            level: "low".to_owned(),
            description: "projection-only".to_owned(),
        });
    }
    packet
}
