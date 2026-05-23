//! Source Registry persistence and host confirmation fixtures.

use std::fs;
use std::path::Path;

use coreroom::config::COREROOM_DIR;
use coreroom::source_registry::{
    load_source_registry, save_source_registry, ProjectSource, RefreshPolicy, SourceKind,
    SourceRegistrationPlan, SourceRegistry, SourceTrustLevel, SOURCE_REGISTRY_FILE,
};

#[test]
fn source_registry_roundtrips_local_repo_doc_and_url_snapshot() {
    let tmp = tempfile::tempdir().expect("tempdir");
    seed_files(tmp.path());

    let mut registry = SourceRegistry::new();
    for source in [
        local_repo_source("vendor/core-api"),
        policy_doc_source("docs/policies/security.md"),
        url_snapshot_source("https://docs.example.test/api"),
    ] {
        let plan = SourceRegistrationPlan::new(source);
        assert!(plan.requires_confirmation);
        assert!(!plan.allows_silent_refresh);
        let confirmed = plan.confirm("user").expect("confirm");
        registry
            .register_confirmed_source(confirmed, tmp.path())
            .expect("register source");
    }

    let path = save_source_registry(tmp.path(), &registry).expect("save");
    assert_eq!(
        path,
        tmp.path().join(COREROOM_DIR).join(SOURCE_REGISTRY_FILE)
    );

    let content = fs::read_to_string(&path).expect("content");
    assert!(content.contains("kind = \"local-repo\""));
    assert!(content.contains("kind = \"policy-doc\""));
    assert!(content.contains("kind = \"url-snapshot\""));
    assert!(content.contains("trustLevel = \"external-doc\""));
    assert!(content.contains("refreshPolicy = \"on-confirmation\""));

    let loaded = load_source_registry(&path, tmp.path()).expect("load");
    assert_eq!(loaded, registry);
}

#[test]
fn source_registry_validation_fails_loudly_for_bad_sources() {
    let tmp = tempfile::tempdir().expect("tempdir");
    seed_files(tmp.path());

    let missing_file = policy_doc_source("docs/policies/missing.md");
    assert!(missing_file
        .validate(tmp.path())
        .expect_err("missing file")
        .to_string()
        .contains("not an accessible file"));

    let mut missing_pin = policy_doc_source("docs/policies/security.md");
    missing_pin.pin.clear();
    assert!(missing_pin
        .validate(tmp.path())
        .expect_err("missing pin")
        .to_string()
        .contains("pin cannot be empty"));

    let inaccessible_repo = local_repo_source("docs");
    assert!(inaccessible_repo
        .validate(tmp.path())
        .expect_err("not git checkout")
        .to_string()
        .contains("not a Git checkout"));

    let invalid_trust = r#"
schemaVersion = 1

[[sources]]
id = "bad-trust"
kind = "policy-doc"
path = "docs/policies/security.md"
pin = "sha256:abc123"
trustLevel = "magic"
owner = "security"
visibleRoles = ["host"]
purpose = "Invalid trust fixture."
refreshPolicy = "manual"
"#;
    let err = toml::from_str::<SourceRegistry>(invalid_trust).expect_err("invalid trust");
    assert!(err.to_string().contains("unknown variant"));
}

#[test]
fn host_source_registration_fixture_documents_confirmation_boundary() {
    let fixture = include_str!("fixtures/source_registry_host_registration.txt");

    assert!(fixture.contains("Source Registry entry proposal"));
    assert!(fixture.contains("Confirmation required: yes"));
    assert!(fixture.contains("Silent refresh allowed: no"));
    assert!(fixture.contains("Role knowledge: no"));
    assert!(fixture.contains("ContextPack: not yet"));
}

fn seed_files(root: &Path) {
    fs::create_dir_all(root.join("vendor/core-api/.git")).expect("core repo");
    fs::create_dir_all(root.join("docs/policies")).expect("policy dir");
    fs::write(
        root.join("docs/policies/security.md"),
        "# Security Policy\n\nUse pinned project sources.\n",
    )
    .expect("policy file");
}

fn local_repo_source(path: &str) -> ProjectSource {
    ProjectSource {
        id: "core-api".to_owned(),
        kind: SourceKind::LocalRepo,
        path: Some(path.to_owned()),
        url: None,
        pin: "commit:0123456789abcdef".to_owned(),
        trust_level: SourceTrustLevel::Internal,
        owner: "platform-team".to_owned(),
        visible_roles: vec!["host".to_owned(), "engineer".to_owned()],
        purpose: "Integration behavior and API contracts.".to_owned(),
        refresh_policy: RefreshPolicy::OnConfirmation,
    }
}

fn policy_doc_source(path: &str) -> ProjectSource {
    ProjectSource {
        id: "security-policy".to_owned(),
        kind: SourceKind::PolicyDoc,
        path: Some(path.to_owned()),
        url: None,
        pin: "sha256:abc123".to_owned(),
        trust_level: SourceTrustLevel::Policy,
        owner: "security".to_owned(),
        visible_roles: vec!["host".to_owned(), "security".to_owned()],
        purpose: "Security constraints for source handling.".to_owned(),
        refresh_policy: RefreshPolicy::Manual,
    }
}

fn url_snapshot_source(url: &str) -> ProjectSource {
    ProjectSource {
        id: "provider-docs".to_owned(),
        kind: SourceKind::UrlSnapshot,
        path: None,
        url: Some(url.to_owned()),
        pin: "snapshot:deadbeef".to_owned(),
        trust_level: SourceTrustLevel::ExternalDoc,
        owner: "host".to_owned(),
        visible_roles: vec!["host".to_owned(), "engineer".to_owned()],
        purpose: "External API reference snapshot.".to_owned(),
        refresh_policy: RefreshPolicy::OnConfirmation,
    }
}
