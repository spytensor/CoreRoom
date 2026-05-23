//! Source graph fixtures.

use std::collections::BTreeMap;

use coreroom::context_pack::{ContextPack, ContextPackEntry};
use coreroom::source_graph::{
    SourceGraph, SourceGraphFindingKind, SourceGraphNodeFacts, SourceRefreshPlan,
};
use coreroom::source_registry::SourceTrustLevel;

#[test]
fn source_graph_fixture_covers_multi_repo_context_and_drift() {
    let graph: SourceGraph =
        toml::from_str(include_str!("fixtures/source_graph_multi_repo.toml")).expect("parse graph");
    graph.validate().expect("valid graph");

    let explanation = graph
        .explain_work_order_sources("WO-0216")
        .expect("host explanation");
    assert!(explanation.contains("Source versions for WO-0216"));
    assert!(explanation.contains("core-api"));
    assert!(explanation.contains("public-api-doc -> core-api"));

    let citations = graph
        .evidence_citations_for_work_order("WO-0216")
        .expect("citations");
    assert!(citations
        .iter()
        .any(|citation| citation.source_id == "core-api" && citation.pin == "commit:core-api-v1"));
    assert!(citations
        .iter()
        .any(|citation| !citation.graph_paths.is_empty()));

    let findings = graph.detect_drift(&current_facts()).expect("drift");
    assert_finding(&findings, "core-api", SourceGraphFindingKind::CommitChanged);
    assert_finding(
        &findings,
        "public-api-doc",
        SourceGraphFindingKind::UrlSnapshotStale,
    );
    assert_finding(
        &findings,
        "security-policy",
        SourceGraphFindingKind::TrustChanged,
    );
    assert_finding(
        &findings,
        "release-checklist",
        SourceGraphFindingKind::MissingSource,
    );
    assert_finding(
        &findings,
        "design-system",
        SourceGraphFindingKind::VisibilityChanged,
    );
}

#[test]
fn context_pack_graph_visibility_denies_roles_outside_source_visibility() {
    let graph: SourceGraph =
        toml::from_str(include_str!("fixtures/source_graph_multi_repo.toml")).expect("parse graph");
    let pack = ContextPack {
        schema_version: 1,
        id: "CTX-WO-0216".to_owned(),
        work_order: "WO-0216".to_owned(),
        entries: vec![
            ContextPackEntry {
                source_id: "core-api".to_owned(),
                path: Some("openapi.yaml".to_owned()),
                range: None,
                snapshot_ref: None,
                reason: "Engineer needs API contract.".to_owned(),
                target_roles: vec!["engineer".to_owned()],
                source_pin: "commit:core-api-v1".to_owned(),
                trust_level: SourceTrustLevel::Internal,
            },
            ContextPackEntry {
                source_id: "security-policy".to_owned(),
                path: Some("docs/policy/security.md".to_owned()),
                range: None,
                snapshot_ref: None,
                reason: "Engineer should not receive policy-only source directly.".to_owned(),
                target_roles: vec!["engineer".to_owned()],
                source_pin: "sha256:security-v1".to_owned(),
                trust_level: SourceTrustLevel::Policy,
            },
        ],
    };

    let report = graph
        .validate_context_pack_visibility(&pack)
        .expect("visibility report");

    assert_finding(
        &report.findings,
        "security-policy",
        SourceGraphFindingKind::VisibilityDenied,
    );
    assert!(!report
        .findings
        .iter()
        .any(|finding| finding.source_id == "core-api"));
}

#[test]
fn source_refresh_plan_requires_explicit_confirmation() {
    let plan = SourceRefreshPlan::new("public-api-doc", "snapshot:docs-v1", "snapshot:docs-v2")
        .expect("plan");

    assert!(plan.requires_confirmation);
    assert!(!plan.allows_silent_refresh);

    let confirmed = plan.confirm("user").expect("confirmed");
    assert_eq!(confirmed.confirmed_by, "user");
}

fn current_facts() -> BTreeMap<String, SourceGraphNodeFacts> {
    BTreeMap::from([
        (
            "app".to_owned(),
            SourceGraphNodeFacts {
                exists: true,
                current_pin: "sha256:app-v1".to_owned(),
                trust_level: SourceTrustLevel::Project,
                visible_roles: vec!["host".to_owned(), "engineer".to_owned(), "qa".to_owned()],
            },
        ),
        (
            "core-api".to_owned(),
            SourceGraphNodeFacts {
                exists: true,
                current_pin: "commit:core-api-v2".to_owned(),
                trust_level: SourceTrustLevel::Internal,
                visible_roles: vec!["host".to_owned(), "engineer".to_owned()],
            },
        ),
        (
            "public-api-doc".to_owned(),
            SourceGraphNodeFacts {
                exists: true,
                current_pin: "snapshot:docs-v2".to_owned(),
                trust_level: SourceTrustLevel::ExternalDoc,
                visible_roles: vec!["host".to_owned(), "engineer".to_owned()],
            },
        ),
        (
            "security-policy".to_owned(),
            SourceGraphNodeFacts {
                exists: true,
                current_pin: "sha256:security-v1".to_owned(),
                trust_level: SourceTrustLevel::Internal,
                visible_roles: vec!["host".to_owned(), "security".to_owned()],
            },
        ),
        (
            "design-system".to_owned(),
            SourceGraphNodeFacts {
                exists: true,
                current_pin: "commit:design-v1".to_owned(),
                trust_level: SourceTrustLevel::Internal,
                visible_roles: vec!["host".to_owned(), "frontend".to_owned()],
            },
        ),
        (
            "deployment-runbook".to_owned(),
            SourceGraphNodeFacts {
                exists: true,
                current_pin: "sha256:runbook-v1".to_owned(),
                trust_level: SourceTrustLevel::Internal,
                visible_roles: vec!["host".to_owned(), "sre".to_owned()],
            },
        ),
        (
            "release-checklist".to_owned(),
            SourceGraphNodeFacts {
                exists: false,
                current_pin: "sha256:release-v1".to_owned(),
                trust_level: SourceTrustLevel::Internal,
                visible_roles: vec!["host".to_owned(), "qa".to_owned()],
            },
        ),
    ])
}

fn assert_finding(
    findings: &[coreroom::source_graph::SourceGraphFinding],
    source_id: &str,
    kind: SourceGraphFindingKind,
) {
    assert!(
        findings
            .iter()
            .any(|finding| finding.source_id == source_id && finding.kind == kind),
        "missing {kind:?} for {source_id}: {findings:?}"
    );
}
