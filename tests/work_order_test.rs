//! WorkOrder persistence and host binding fixtures.

use std::fs;

use coreroom::config::COREROOM_DIR;
use coreroom::work_order::{
    load_work_order, save_work_order, HostIntentClassification, RequiredEvidence, WorkOrder,
    WorkOrderDraft, WorkOrderStatus, WORK_ORDERS_DIR,
};

#[test]
fn work_order_roundtrips_under_coreroom_work_orders() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut draft = WorkOrderDraft::new(
        "WO-0207",
        "Define a WorkOrder model and bind it to GitHub Issue #207.",
        HostIntentClassification::PersistentWorkorder,
    );
    draft.title = Some("WorkOrder model and GitHub binding".to_owned());
    draft.phase = Some("v0.6.0 - Engineering Control Room".to_owned());
    draft.epic = Some("WorkOrder / GitHub Binding".to_owned());
    draft.tracker_issue = Some(202);
    draft.tracker_checkbox = Some("#207 - WorkOrder model and GitHub binding".to_owned());
    draft.acceptance_criteria = vec![
        "Define WorkOrder fields".to_owned(),
        "Bind existing GitHub Issue without mutating issue body".to_owned(),
    ];
    draft.required_evidence = vec![
        RequiredEvidence::ChangedFiles,
        RequiredEvidence::Validation,
        RequiredEvidence::TrackerUpdate,
    ];

    let mut work_order = WorkOrder::draft_from_host_intake(draft).expect("draft");
    let plan = WorkOrder::plan_existing_issue_binding(207).expect("binding plan");
    let confirmed = plan.confirm("user").expect("confirmation");
    work_order
        .apply_confirmed_issue_binding(&confirmed)
        .expect("bind existing issue");
    work_order.status = WorkOrderStatus::Ready;

    let path = save_work_order(tmp.path(), &work_order).expect("save");
    assert_eq!(
        path,
        tmp.path()
            .join(COREROOM_DIR)
            .join(WORK_ORDERS_DIR)
            .join("WO-0207.toml")
    );

    let content = fs::read_to_string(&path).expect("content");
    assert!(content.contains("githubIssue = 207"));
    assert!(content.contains("trackerIssue = 202"));
    assert!(content.contains(r#"status = "ready""#));

    let loaded = load_work_order(&path).expect("load");
    assert_eq!(loaded, work_order);
}

#[test]
fn host_binding_fixture_documents_confirmation_boundary() {
    let fixture = include_str!("fixtures/workorder_host_binding.txt");

    assert!(fixture.contains("Classification: persistent-workorder"));
    assert!(fixture.contains("Confirmation required: yes"));
    assert!(fixture.contains("Binding plan: bind existing GitHub Issue #207"));
    assert!(fixture.contains("Issue body mutation: no"));
}
