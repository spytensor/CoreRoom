use super::*;
use pretty_assertions::assert_eq;
use std::fs;
use tempfile::TempDir;

/// Build a `.coderoom/` skeleton with one role (host) so the
/// commands have a valid starting point.
fn fixture() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let coderoom = tmp.path().join(CODEROOM_DIR);
    fs::create_dir_all(coderoom.join(ROLES_DIR)).unwrap();
    fs::write(
        coderoom.join(CONFIG_FILE),
        r#"
default_engine = "cc"
host_role = "host"

[roles.host]
"#,
    )
    .unwrap();
    fs::write(coderoom.join(ROLES_DIR).join("host.md"), "host priors").unwrap();
    tmp
}

#[test]
fn add_creates_role_entry_and_priors_file() {
    let tmp = fixture();
    add(tmp.path(), "backend", None, None).unwrap();

    let coderoom = tmp.path().join(CODEROOM_DIR);
    let cfg = Config::load_test(tmp.path()).unwrap();
    assert!(cfg.roles.contains_key("backend"));
    let priors = fs::read_to_string(
        coderoom
            .join(ROLES_DIR)
            .join("backend")
            .join(crate::manifest::ROLE_PRIORS_FILE),
    )
    .unwrap();
    assert!(priors.contains("@backend"));
    assert!(priors.contains("@host"));
    assert!(!priors.contains("{ROLE}"));
    assert!(!priors.contains("{HOST}"));
    assert!(!priors.contains("{PEERS}"));
}

#[test]
fn add_persists_engine_and_model_overrides() {
    let tmp = fixture();
    add(tmp.path(), "security", Some(Engine::Codex), Some("o3")).unwrap();
    let cfg = Config::load_test(tmp.path()).unwrap();
    let entry = cfg.roles.get("security").unwrap();
    assert_eq!(entry.engine, Some(Engine::Codex));
    assert_eq!(entry.model.as_deref(), Some("o3"));
}

#[test]
fn attach_migrates_legacy_priors_and_writes_manifest() {
    let tmp = fixture();
    let source = tmp.path().join("runbook.md");
    fs::write(&source, "DEPLOYMENT_RUNBOOK").unwrap();

    attach(tmp.path(), "host", &source, Some("deployment.md")).unwrap();

    let coderoom = tmp.path().join(CODEROOM_DIR);
    assert!(!coderoom.join(ROLES_DIR).join("host.md").exists());
    assert!(coderoom
        .join(ROLES_DIR)
        .join("host")
        .join(crate::manifest::ROLE_PRIORS_FILE)
        .is_file());
    let knowledge_path = coderoom
        .join(ROLES_DIR)
        .join("host")
        .join(crate::manifest::KNOWLEDGE_DIR)
        .join("deployment.md");
    assert_eq!(
        fs::read_to_string(&knowledge_path).unwrap(),
        "DEPLOYMENT_RUNBOOK"
    );
    let manifest = crate::manifest::read_manifest(&coderoom.join(ROLES_DIR).join("host")).unwrap();
    assert_eq!(manifest.files.len(), 1);
    assert_eq!(manifest.files[0].name, "deployment.md");
    assert_eq!(
        manifest.files[0].sha256,
        crate::manifest::sha256_file(&knowledge_path).unwrap()
    );
}

#[test]
fn detach_removes_manifest_entry_and_file() {
    let tmp = fixture();
    let source = tmp.path().join("runbook.md");
    fs::write(&source, "DEPLOYMENT_RUNBOOK").unwrap();
    attach(tmp.path(), "host", &source, Some("deployment.md")).unwrap();

    detach(tmp.path(), "host", "deployment.md").unwrap();

    let coderoom = tmp.path().join(CODEROOM_DIR);
    let role_dir = coderoom.join(ROLES_DIR).join("host");
    assert!(!role_dir
        .join(crate::manifest::KNOWLEDGE_DIR)
        .join("deployment.md")
        .exists());
    let manifest = crate::manifest::read_manifest(&role_dir).unwrap();
    assert!(manifest.files.is_empty());
}

#[test]
fn set_owner_persists_role_owner() {
    let tmp = fixture();
    set_owner(tmp.path(), "host", "alice@example.com").unwrap();
    let cfg = Config::load_test(tmp.path()).unwrap();
    assert_eq!(
        cfg.roles["host"].owner.as_deref(),
        Some("alice@example.com")
    );
}

#[test]
fn set_authority_persists_deduped_scopes() {
    let tmp = fixture();
    set_authority(
        tmp.path(),
        "host",
        &[
            AuthorityScope::Infra,
            AuthorityScope::Deployment,
            AuthorityScope::Infra,
        ],
    )
    .unwrap();
    let cfg = Config::load_test(tmp.path()).unwrap();
    assert_eq!(
        cfg.roles["host"].authority,
        vec![AuthorityScope::Deployment, AuthorityScope::Infra]
    );
}

#[test]
fn add_many_creates_roles_in_one_loadable_batch() {
    let tmp = fixture();
    let added = add_many(
        tmp.path(),
        &[
            RoleAddition {
                name: "backend".into(),
                engine: None,
                model: None,
            },
            RoleAddition {
                name: "security".into(),
                engine: Some(Engine::Codex),
                model: None,
            },
        ],
    )
    .unwrap();

    assert_eq!(added, 2);
    let cfg = Config::load_test(tmp.path()).unwrap();
    assert!(cfg.roles.contains_key("backend"));
    assert_eq!(cfg.roles["security"].engine, Some(Engine::Codex));
    assert!(tmp
        .path()
        .join(CODEROOM_DIR)
        .join(ROLES_DIR)
        .join("backend")
        .join(crate::manifest::ROLE_PRIORS_FILE)
        .is_file());
    assert!(tmp
        .path()
        .join(CODEROOM_DIR)
        .join(ROLES_DIR)
        .join("security")
        .join(crate::manifest::ROLE_PRIORS_FILE)
        .is_file());
}

#[test]
fn add_many_skips_existing_roles() {
    let tmp = fixture();
    let added = add_many(
        tmp.path(),
        &[RoleAddition {
            name: "host".into(),
            engine: None,
            model: None,
        }],
    )
    .unwrap();

    assert_eq!(added, 0);
}

#[test]
fn add_many_preserves_existing_config_text() {
    let tmp = fixture();
    let config_path = tmp.path().join(CODEROOM_DIR).join(CONFIG_FILE);
    fs::write(
        &config_path,
        r#"# keep this comment
default_engine = "cc"
host_role = "host"

[roles.host]
"#,
    )
    .unwrap();

    add_many(
        tmp.path(),
        &[RoleAddition {
            name: "backend".into(),
            engine: None,
            model: None,
        }],
    )
    .unwrap();

    let body = fs::read_to_string(config_path).unwrap();
    assert!(body.contains("# keep this comment"));
    assert!(body.contains("[roles.backend]"));
}

#[test]
fn add_refuses_duplicate_role() {
    let tmp = fixture();
    add(tmp.path(), "backend", None, None).unwrap();
    let err = add(tmp.path(), "backend", None, None).expect_err("duplicate add");
    assert!(err.to_string().contains("already exists"));
}

#[test]
fn add_validates_name() {
    let tmp = fixture();
    let err = add(tmp.path(), "@backend", None, None).expect_err("leading @");
    assert!(err.to_string().contains('@'));

    let err = add(tmp.path(), "1bad", None, None).expect_err("starts with digit");
    assert!(err.to_string().contains("ASCII letter"));

    let err = add(tmp.path(), "with spaces", None, None).expect_err("space");
    assert!(err.to_string().contains("invalid characters"));
}

#[test]
fn rm_removes_role_and_priors() {
    let tmp = fixture();
    add(tmp.path(), "backend", None, None).unwrap();
    rm(tmp.path(), "backend").unwrap();
    let cfg = Config::load_test(tmp.path()).unwrap();
    assert!(!cfg.roles.contains_key("backend"));
    assert!(!tmp
        .path()
        .join(CODEROOM_DIR)
        .join(ROLES_DIR)
        .join("backend")
        .exists());
}

#[test]
fn rm_refuses_to_remove_host() {
    let tmp = fixture();
    let err = rm(tmp.path(), "host").expect_err("host should be protected");
    assert!(err.to_string().contains("host"));
}

#[test]
fn rm_unknown_role_errors() {
    let tmp = fixture();
    let err = rm(tmp.path(), "ghost").expect_err("unknown role");
    assert!(err.to_string().contains("ghost"));
}

#[test]
fn set_host_persists_host_role() {
    let tmp = fixture();
    add(tmp.path(), "backend", None, None).unwrap();
    set_host(tmp.path(), "backend").unwrap();
    let cfg = Config::load_test(tmp.path()).unwrap();
    assert_eq!(cfg.host_role, "backend");
}
