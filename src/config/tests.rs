use super::*;
use pretty_assertions::assert_eq;
use std::fs;
use tempfile::TempDir;

/// Build a minimal, valid `.coreroom/` tree with `pm` and `backend` roles.
fn fixture(toml_body: &str) -> TempDir {
    let tmp = TempDir::new().unwrap();
    let coreroom_dir = tmp.path().join(COREROOM_DIR);
    fs::create_dir_all(coreroom_dir.join(ROLES_DIR)).unwrap();
    fs::write(coreroom_dir.join(CONFIG_FILE), toml_body).unwrap();
    fs::write(coreroom_dir.join(ROLES_DIR).join("pm.md"), "pm priors\n").unwrap();
    fs::write(
        coreroom_dir.join(ROLES_DIR).join("backend.md"),
        "backend priors\n",
    )
    .unwrap();
    tmp
}

#[test]
fn load_minimal_valid_config() {
    let tmp = fixture(
        r#"
default_engine = "cc"
host_role = "pm"

[roles.pm]
[roles.backend]
"#,
    );

    let cfg = Config::load_test(tmp.path()).expect("load");
    assert_eq!(cfg.default_engine, Engine::Cc);
    assert_eq!(cfg.permission_mode, PermissionMode::Ask);
    assert_eq!(cfg.host_role, "pm");
    assert_eq!(cfg.roles.len(), 2);
    assert!(cfg.is_host("pm"));
    assert!(!cfg.is_host("backend"));
    assert_eq!(cfg.roles["backend"].access, None);
    assert_eq!(cfg.effective_role_access("pm"), RoleAccess::HostControl);
    assert_eq!(cfg.effective_role_access("backend"), RoleAccess::ReadReview);
}

#[test]
fn role_config_inherits_defaults() {
    let tmp = fixture(
        r#"
default_engine = "cc"
default_model = "opus"
host_role = "pm"

[roles.pm]
[roles.backend]
"#,
    );
    let cfg = Config::load_test(tmp.path()).unwrap();
    let coreroom = tmp.path().join(COREROOM_DIR);

    let pm = cfg.role_config("pm", &coreroom).unwrap();
    assert_eq!(pm.name, "pm");
    assert_eq!(pm.engine, Engine::Cc);
    assert_eq!(pm.model.as_deref(), Some("opus"));
    assert_eq!(pm.permission_mode, PermissionMode::Ask);

    let backend = cfg.role_config("backend", &coreroom).unwrap();
    assert_eq!(backend.engine, Engine::Cc); // inherited
    assert_eq!(backend.model.as_deref(), Some("opus")); // inherited
    assert_eq!(backend.permission_mode, PermissionMode::Ask); // inherited
}

#[test]
fn role_config_overrides_engine_and_model() {
    let tmp = fixture(
        r#"
default_engine = "cc"
default_model = "opus"
permission_mode = "auto"
host_role = "pm"

[roles.pm]
[roles.backend]

[roles.security]
engine = "codex"
model = "o3"
permission_mode = "bypass"
"#,
    );
    let coreroom = tmp.path().join(COREROOM_DIR);
    // create the security priors so validation passes
    fs::write(
        coreroom.join(ROLES_DIR).join("security.md"),
        "security priors\n",
    )
    .unwrap();

    let cfg = Config::load_test(tmp.path()).unwrap();
    let security = cfg.role_config("security", &coreroom).unwrap();
    assert_eq!(security.engine, Engine::Codex);
    assert_eq!(security.model.as_deref(), Some("o3"));
    assert_eq!(security.permission_mode, PermissionMode::Bypass);
}

#[test]
fn role_entry_parses_owner_access_and_authority() {
    let tmp = fixture(
        r#"
default_engine = "cc"
host_role = "pm"

[roles.pm]

[roles.backend]
owner = "alice@example.com"
access = "write"
authority = ["deployment", "infra", "secrets"]
"#,
    );

    let cfg = Config::load_test(tmp.path()).unwrap();
    let backend = cfg.roles.get("backend").unwrap();
    assert_eq!(backend.owner.as_deref(), Some("alice@example.com"));
    assert_eq!(backend.access, Some(RoleAccess::Write));
    assert_eq!(cfg.effective_role_access("backend"), RoleAccess::Write);
    assert_eq!(
        backend.authority,
        vec![
            AuthorityScope::Deployment,
            AuthorityScope::Infra,
            AuthorityScope::Secrets,
        ]
    );
}

#[test]
fn effective_role_access_defaults_host_engineer_and_specialists() {
    let tmp = fixture(
        r#"
default_engine = "cc"
host_role = "host"

[roles.host]

[roles.engineer]

[roles.backend]

[roles.reviewer]
access = "write"
"#,
    );
    let coreroom = tmp.path().join(COREROOM_DIR);
    fs::write(coreroom.join(ROLES_DIR).join("host.md"), "host\n").unwrap();
    fs::write(coreroom.join(ROLES_DIR).join("engineer.md"), "engineer\n").unwrap();
    fs::write(coreroom.join(ROLES_DIR).join("reviewer.md"), "reviewer\n").unwrap();

    let cfg = Config::load_test(tmp.path()).unwrap();
    assert_eq!(cfg.effective_role_access("host"), RoleAccess::HostControl);
    assert_eq!(cfg.effective_role_access("engineer"), RoleAccess::Write);
    assert_eq!(cfg.effective_role_access("backend"), RoleAccess::ReadReview);
    assert_eq!(cfg.effective_role_access("reviewer"), RoleAccess::Write);
}

#[test]
fn role_access_is_separate_from_permission_mode() {
    let tmp = fixture(
        r#"
default_engine = "cc"
permission_mode = "bypass"
host_role = "pm"

[roles.pm]

[roles.backend]
access = "read-review"
permission_mode = "ask"
"#,
    );
    let coreroom = tmp.path().join(COREROOM_DIR);
    let cfg = Config::load_test(tmp.path()).unwrap();
    let backend = cfg.role_config("backend", &coreroom).unwrap();

    assert_eq!(backend.permission_mode, PermissionMode::Ask);
    assert_eq!(cfg.effective_role_access("backend"), RoleAccess::ReadReview);
}

#[test]
fn unknown_role_access_is_rejected_loudly() {
    let tmp = fixture(
        r#"
default_engine = "cc"
host_role = "pm"

[roles.pm]

[roles.backend]
access = "superuser"
"#,
    );

    let err = Config::load_test(tmp.path()).expect_err("unknown access should fail parse");
    match err {
        ConfigError::Parse { source, .. } => {
            assert!(source.to_string().contains("superuser"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn unknown_authority_scope_is_rejected_loudly() {
    let tmp = fixture(
        r#"
default_engine = "cc"
host_role = "pm"

[roles.pm]

[roles.backend]
authority = ["infra", "foobar"]
"#,
    );

    let err = Config::load_test(tmp.path()).expect_err("unknown scope should fail parse");
    match err {
        ConfigError::Parse { source, .. } => {
            assert!(source.to_string().contains("foobar"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn codex_role_without_permission_override_uses_bypass() {
    let tmp = fixture(
        r#"
default_engine = "cc"
permission_mode = "ask"
host_role = "pm"

[roles.pm]

[roles.security]
engine = "codex"
"#,
    );
    let coreroom = tmp.path().join(COREROOM_DIR);
    fs::write(
        coreroom.join(ROLES_DIR).join("security.md"),
        "security priors\n",
    )
    .unwrap();

    let cfg = Config::load_test(tmp.path()).unwrap();
    let security = cfg.role_config("security", &coreroom).unwrap();
    assert_eq!(security.engine, Engine::Codex);
    assert_eq!(security.permission_mode, PermissionMode::Bypass);
}

#[test]
fn explicit_codex_permission_mode_is_preserved() {
    let tmp = fixture(
        r#"
default_engine = "cc"
permission_mode = "bypass"
host_role = "pm"

[roles.pm]

[roles.security]
engine = "codex"
permission_mode = "ask"
"#,
    );
    let coreroom = tmp.path().join(COREROOM_DIR);
    fs::write(
        coreroom.join(ROLES_DIR).join("security.md"),
        "security priors\n",
    )
    .unwrap();

    let cfg = Config::load_test(tmp.path()).unwrap();
    let security = cfg.role_config("security", &coreroom).unwrap();
    assert_eq!(security.engine, Engine::Codex);
    assert_eq!(security.permission_mode, PermissionMode::Ask);
}

#[test]
fn missing_host_role_is_rejected() {
    let tmp = fixture(
        r#"
default_engine = "cc"
host_role = "ghost"

[roles.pm]
[roles.backend]
"#,
    );
    let err = Config::load_test(tmp.path()).expect_err("should reject missing host_role");
    match err {
        ConfigError::MissingHostRole { host, declared } => {
            assert_eq!(host, "ghost");
            assert_eq!(declared, vec!["backend".to_owned(), "pm".to_owned()]);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn missing_priors_file_is_rejected() {
    let tmp = fixture(
        r#"
default_engine = "cc"
host_role = "pm"

[roles.pm]
[roles.backend]
[roles.frontend]
"#,
    );
    // frontend declared in config but no .md file — should fail.
    let err = Config::load_test(tmp.path()).expect_err("should reject missing priors");
    match err {
        ConfigError::MissingPriors { role, .. } => {
            assert_eq!(role, "frontend");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn missing_config_file_surfaces_io_error() {
    let tmp = TempDir::new().unwrap();
    // don't even create .coreroom/
    match Config::load_test(tmp.path()).expect_err("missing config should error") {
        ConfigError::Read { .. } => {}
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn role_names_returns_all_declared_roles() {
    let tmp = fixture(
        r#"
default_engine = "cc"
host_role = "pm"

[roles.pm]
[roles.backend]
"#,
    );
    let cfg = Config::load_test(tmp.path()).unwrap();
    let mut names: Vec<&str> = cfg.role_names().collect();
    names.sort_unstable();
    assert_eq!(names, vec!["backend", "pm"]);
}
