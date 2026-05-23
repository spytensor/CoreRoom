//! v0.6 host-led Engineering Control Room dogfood validation.

use std::fs;
use std::path::Path;

use coreroom::config::COREROOM_DIR;
use coreroom::context_pack::{save_context_pack, ContextPack, CONTEXT_PACK_SCHEMA_VERSION};
use coreroom::evidence_packet::{save_evidence_packet, EvidencePacket, EvidenceStatus};
use coreroom::source_registry::{save_source_registry, SourceRegistry};
use coreroom::tracker::{detect_tracker_mismatch, TrackerWorkState};
use coreroom::work_order::{save_work_order, WorkOrder, WorkOrderStatus};

#[test]
fn v06_host_led_dogfood_flow_is_structurally_complete() {
    let tmp = tempfile::tempdir().expect("tempdir");
    setup_project_sources(tmp.path());

    let work_order: WorkOrder =
        toml::from_str(include_str!("fixtures/v06_dogfood_work_order.toml"))
            .expect("work order fixture");
    let registry: SourceRegistry =
        toml::from_str(include_str!("fixtures/v06_dogfood_source_registry.toml"))
            .expect("source registry fixture");
    let context_pack: ContextPack =
        toml::from_str(include_str!("fixtures/v06_dogfood_context_pack.toml"))
            .expect("context pack fixture");
    let evidence: EvidencePacket =
        toml::from_str(include_str!("fixtures/v06_dogfood_evidence_packet.toml"))
            .expect("evidence packet fixture");

    assert_eq!(work_order.status, WorkOrderStatus::Closed);
    assert_eq!(work_order.github_issue, Some(212));
    assert_eq!(work_order.pull_request, Some(228));
    assert_eq!(work_order.tracker_issue, Some(202));
    assert_eq!(work_order.acceptance_criteria.len(), 10);

    registry.validate(tmp.path()).expect("registry validates");
    let context_validation = context_pack
        .validate_against_registry(&registry)
        .expect("context pack validates");
    assert!(
        context_validation.warnings.is_empty(),
        "dogfood context should not start stale: {:?}",
        context_validation.warnings
    );
    assert_eq!(context_pack.schema_version, CONTEXT_PACK_SCHEMA_VERSION);
    assert_eq!(context_pack.work_order, "WO-0212");
    assert!(context_pack
        .entries
        .iter()
        .any(|entry| entry.target_roles == ["engineer"]));
    assert!(context_pack
        .entries
        .iter()
        .any(|entry| entry.target_roles == ["reviewer", "qa"]));

    let evidence_report = evidence.completion_report().expect("evidence report");
    assert_eq!(evidence_report.status, EvidenceStatus::Complete);
    assert!(evidence_report.missing.is_empty());
    assert_eq!(evidence.github_issue, 212);
    assert_eq!(evidence.pull_request, Some(228));
    assert!(evidence
        .role_reviews
        .iter()
        .any(|review| review.role == "engineer"));
    assert!(evidence
        .role_reviews
        .iter()
        .any(|review| review.role == "reviewer"));

    let work_path = save_work_order(tmp.path(), &work_order).expect("save work order");
    let registry_path = save_source_registry(tmp.path(), &registry).expect("save registry");
    let context_path =
        save_context_pack(tmp.path(), &registry, &context_pack).expect("save context pack");
    let evidence_path = save_evidence_packet(tmp.path(), &evidence).expect("save evidence");

    assert!(work_path.ends_with(".coreroom/work-orders/WO-0212.toml"));
    assert_eq!(
        registry_path,
        tmp.path().join(COREROOM_DIR).join("source-registry.toml")
    );
    assert!(context_path.ends_with(".coreroom/context-packs/CTX-WO-0212.toml"));
    assert!(evidence_path.ends_with(".coreroom/evidence/WO-0212.toml"));

    let tracker_report = detect_tracker_mismatch(
        v06_tracker_closed_body(),
        &TrackerWorkState {
            issue: 212,
            issue_closed: true,
            pr_merged: true,
            evidence_packet_exists: true,
        },
    );
    assert!(
        tracker_report.is_clean(),
        "closed dogfood tracker should be clean: {:?}",
        tracker_report.findings
    );

    assert_host_transcript_covers_acceptance_criteria();
}

fn setup_project_sources(root: &Path) {
    fs::create_dir_all(root.join("docs")).expect("docs dir");
    fs::create_dir_all(root.join(".github")).expect("github dir");
    fs::write(
        root.join("docs").join("architecture.md"),
        "# Architecture\n\nWrapper, not runtime.\n",
    )
    .expect("architecture");
    fs::write(
        root.join("docs").join("sdlc-gates.md"),
        "# SDLC Gates\n\nintake -> discovery -> plan -> review -> signoff -> implement -> qa -> closed\n",
    )
    .expect("sdlc");
    fs::write(
        root.join(".github").join("PULL_REQUEST_TEMPLATE.md"),
        "## Evidence Packet\n\n## Tracker update\n",
    )
    .expect("pr template");
}

fn assert_host_transcript_covers_acceptance_criteria() {
    let transcript = include_str!("fixtures/v06_host_led_dogfood.txt");

    for required in [
        "User:",
        "Classification: persistent-workorder",
        "WorkOrder draft: WO-0212",
        "Required sources:",
        "ContextPack: CTX-WO-0212",
        "Delegate @engineer",
        "Delegate @reviewer",
        "Evidence Packet: WO-0212",
        "GitHub Issue: #212",
        "Pull Request: #228",
        "Tracker closure: #202 / #212",
        "Remaining risks:",
        "Rollback:",
    ] {
        assert!(transcript.contains(required), "missing `{required}`");
    }

    let signoff = transcript
        .find("Gate phase: signoff")
        .expect("signoff phase");
    let implement = transcript
        .find("Gate phase: implement")
        .expect("implement phase");
    assert!(
        signoff < implement,
        "gate must reach signoff before implementation"
    );
}

fn v06_tracker_closed_body() -> &'static str {
    r"
## Phase 4 - Evidence / Tracker Closure

- [x] #212 - v0.6 end-to-end dogfood validation

## Milestone Acceptance Criteria

- [x] M-AC-10: Dogfood proves host-led request -> issue -> context -> gate -> PR/evidence -> tracker closure.

## Evidence Ledger

| Issue | PR | Status | Evidence | Tracker Updated |
|---|---:|---|---|---|
| #212 | #228 | merged | PR #228; local validation: `cargo test --test v06_host_led_dogfood --quiet`; changed files: v0.6 dogfood fixtures, dogfood test, TASKS, CHANGELOG | yes |
"
}
