//! WorkOrder persistence and host binding fixtures.

use std::collections::HashMap;
use std::fs;

use coreroom::adapter::{Engine, PermissionMode};
use coreroom::config::{Config, RoleEntry, COREROOM_DIR};
use coreroom::work_order::{
    load_work_order, save_work_order, HostIntentClassification, RequiredEvidence, WorkOrder,
    WorkOrderDraft, WorkOrderRoleAccess, WorkOrderRoleGrant, WorkOrderStatus, WORK_ORDERS_DIR,
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
    work_order.role_grants = vec![
        grant(
            "backend",
            WorkOrderRoleAccess::Write,
            &["src/work_order.rs", "tests/work_order_test.rs"],
            "issue:#362",
            "Implement scoped WorkOrder grant support.",
        ),
        grant(
            "frontend",
            WorkOrderRoleAccess::Write,
            &["src/console_views.rs"],
            "issue:#362",
            "Surface scoped escalations in console status views.",
        ),
        grant(
            "reviewer",
            WorkOrderRoleAccess::ReadReview,
            &[],
            "issue:#362",
            "Review only; no implementation access.",
        ),
    ];
    work_order
        .validate_with_config(&configured_roles())
        .expect("configured role grants");

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
    assert!(content.contains("[[roleGrants]]"));
    assert!(content.contains(r#"role = "backend""#));
    assert!(content.contains(r#"access = "write""#));
    assert!(content.contains(r#"access = "read-review""#));

    let loaded = load_work_order(&path).expect("load");
    assert_eq!(loaded, work_order);
}

#[test]
fn work_order_role_grants_require_write_scope_and_auditable_source() {
    let mut draft = WorkOrderDraft::new(
        "WO-0362",
        "Model WorkOrder-scoped role grants.",
        HostIntentClassification::PersistentWorkorder,
    );
    draft.acceptance_criteria = vec!["validate grants".to_owned()];
    let mut work_order = WorkOrder::draft_from_host_intake(draft).expect("draft");
    work_order.role_grants = vec![grant(
        "backend",
        WorkOrderRoleAccess::Write,
        &[],
        "issue:#362",
        "Needs implementation access.",
    )];

    let err = work_order.validate().expect_err("write scope required");
    assert!(err.to_string().contains("write role grants require"));

    work_order.role_grants = vec![grant(
        "backend",
        WorkOrderRoleAccess::Write,
        &["src/work_order.rs"],
        "",
        "Needs implementation access.",
    )];

    let err = work_order.validate().expect_err("source required");
    assert!(err.to_string().contains("roleGrants.source"));
}

#[test]
fn work_order_role_grants_validate_against_configured_roles() {
    let mut draft = WorkOrderDraft::new(
        "WO-0362",
        "Validate role grants against configured roles.",
        HostIntentClassification::PersistentWorkorder,
    );
    draft.acceptance_criteria = vec!["configured roles".to_owned()];
    let mut work_order = WorkOrder::draft_from_host_intake(draft).expect("draft");
    work_order.role_grants = vec![grant(
        "security",
        WorkOrderRoleAccess::ReadReview,
        &[],
        "issue:#362",
        "Review only.",
    )];

    let err = work_order
        .validate_with_config(&configured_roles())
        .expect_err("unknown role rejected");
    assert!(err.to_string().contains("security"));
}

#[test]
fn host_binding_fixture_documents_confirmation_boundary() {
    let fixture = include_str!("fixtures/workorder_host_binding.txt");

    assert!(fixture.contains("Classification: persistent-workorder"));
    assert!(fixture.contains("Confirmation required: yes"));
    assert!(fixture.contains("Binding plan: bind existing GitHub Issue #207"));
    assert!(fixture.contains("Issue body mutation: no"));
}

fn grant(
    role: &str,
    access: WorkOrderRoleAccess,
    scopes: &[&str],
    source: &str,
    reason: &str,
) -> WorkOrderRoleGrant {
    WorkOrderRoleGrant {
        role: role.to_owned(),
        access,
        scopes: scopes.iter().map(|scope| (*scope).to_owned()).collect(),
        source: source.to_owned(),
        reason: reason.to_owned(),
    }
}

fn configured_roles() -> Config {
    Config {
        default_engine: Engine::Cc,
        default_model: Some("sonnet".to_owned()),
        permission_mode: PermissionMode::Ask,
        host_role: "host".to_owned(),
        roles: HashMap::from([
            ("host".to_owned(), RoleEntry::default()),
            ("backend".to_owned(), RoleEntry::default()),
            ("frontend".to_owned(), RoleEntry::default()),
            ("reviewer".to_owned(), RoleEntry::default()),
        ]),
    }
}
