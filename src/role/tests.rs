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
    let priors = fs::read_to_string(coderoom.join(ROLES_DIR).join("backend.md")).unwrap();
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
        .join("backend.md")
        .is_file());
    assert!(tmp
        .path()
        .join(CODEROOM_DIR)
        .join(ROLES_DIR)
        .join("security.md")
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
        .join("backend.md")
        .is_file());
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
