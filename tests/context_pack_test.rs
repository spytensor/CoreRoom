//! ContextPack persistence and host delegation fixtures.

use std::fs;

use coreroom::config::COREROOM_DIR;
use coreroom::context_pack::{
    load_context_pack, save_context_pack, ContextPack, ContextPackEntry, ContextPackProposal,
    ContextRange, CONTEXT_PACKS_DIR,
};
use coreroom::source_registry::{
    ProjectSource, RefreshPolicy, SourceKind, SourceRegistry, SourceTrustLevel,
};

#[test]
fn context_pack_roundtrips_different_role_slices() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let registry = registry();
    let pack = context_pack();

    let proposal = ContextPackProposal::new(pack.clone());
    assert!(proposal.requires_confirmation);
    let confirmed = proposal.confirm("user").expect("confirmation");
    let confirmed_pack = confirmed.proposal.context_pack;

    let path = save_context_pack(tmp.path(), &registry, &confirmed_pack).expect("save");
    assert_eq!(
        path,
        tmp.path()
            .join(COREROOM_DIR)
            .join(CONTEXT_PACKS_DIR)
            .join("CTX-WO-0209.toml")
    );

    let content = fs::read_to_string(&path).expect("content");
    assert!(content.contains("workOrder = \"WO-0209\""));
    assert!(content.contains("sourceId = \"core-api\""));
    assert!(content.contains("targetRoles = [\"engineer\"]"));
    assert!(content.contains("targetRoles = [\"security\"]"));

    let loaded = load_context_pack(&path, &registry).expect("load");
    assert_eq!(loaded, confirmed_pack);
}

#[test]
fn context_pack_warns_for_stale_or_unpinned_sources() {
    let mut registry = registry();
    registry.sources[0].pin = "commit:new".to_owned();
    registry.sources[1].pin.clear();

    let validation = context_pack()
        .validate_against_registry(&registry)
        .expect("warnings only");

    assert!(validation
        .warnings
        .iter()
        .any(|warning| warning.contains("pin is stale")));
    assert!(validation
        .warnings
        .iter()
        .any(|warning| warning.contains("unpinned")));
}

#[test]
fn host_context_pack_fixture_documents_scoped_delegation() {
    let fixture = include_str!("fixtures/context_pack_host_delegation.txt");

    assert!(fixture.contains("ContextPack proposal: CTX-WO-0209"));
    assert!(fixture.contains("@engineer receives"));
    assert!(fixture.contains("@security receives"));
    assert!(fixture.contains("No role receives every source by default"));
}

fn registry() -> SourceRegistry {
    SourceRegistry {
        schema_version: coreroom::source_registry::SOURCE_REGISTRY_SCHEMA_VERSION,
        sources: vec![
            ProjectSource {
                id: "core-api".to_owned(),
                kind: SourceKind::LocalRepo,
                path: Some("../core-api".to_owned()),
                url: None,
                pin: "commit:abc123".to_owned(),
                trust_level: SourceTrustLevel::Internal,
                owner: "platform-team".to_owned(),
                visible_roles: vec!["host".to_owned(), "engineer".to_owned()],
                purpose: "Integration behavior and API contracts.".to_owned(),
                refresh_policy: RefreshPolicy::OnConfirmation,
            },
            ProjectSource {
                id: "security-policy".to_owned(),
                kind: SourceKind::PolicyDoc,
                path: Some("docs/policies/security.md".to_owned()),
                url: None,
                pin: "sha256:def456".to_owned(),
                trust_level: SourceTrustLevel::Policy,
                owner: "security".to_owned(),
                visible_roles: vec!["host".to_owned(), "security".to_owned()],
                purpose: "Security constraints.".to_owned(),
                refresh_policy: RefreshPolicy::Manual,
            },
        ],
    }
}

fn context_pack() -> ContextPack {
    ContextPack {
        schema_version: coreroom::context_pack::CONTEXT_PACK_SCHEMA_VERSION,
        id: "CTX-WO-0209".to_owned(),
        work_order: "WO-0209".to_owned(),
        entries: vec![
            ContextPackEntry {
                source_id: "core-api".to_owned(),
                path: Some("src/contracts.rs".to_owned()),
                range: Some(ContextRange {
                    start_line: 10,
                    end_line: 40,
                }),
                snapshot_ref: None,
                reason: "Engineer needs API contract definitions.".to_owned(),
                target_roles: vec!["engineer".to_owned()],
                source_pin: "commit:abc123".to_owned(),
                trust_level: SourceTrustLevel::Internal,
            },
            ContextPackEntry {
                source_id: "security-policy".to_owned(),
                path: Some("docs/policies/security.md".to_owned()),
                range: None,
                snapshot_ref: None,
                reason: "Security needs policy constraints.".to_owned(),
                target_roles: vec!["security".to_owned()],
                source_pin: "sha256:def456".to_owned(),
                trust_level: SourceTrustLevel::Policy,
            },
        ],
    }
}
