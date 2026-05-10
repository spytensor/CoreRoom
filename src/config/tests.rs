use super::*;
use pretty_assertions::assert_eq;
use std::fs;
use tempfile::TempDir;

/// Build a minimal, valid `.coderoom/` tree with `pm` and `backend` roles.
fn fixture(toml_body: &str) -> TempDir {
    let tmp = TempDir::new().unwrap();
    let coderoom_dir = tmp.path().join(CODEROOM_DIR);
    fs::create_dir_all(coderoom_dir.join(ROLES_DIR)).unwrap();
    fs::write(coderoom_dir.join(CONFIG_FILE), toml_body).unwrap();
    fs::write(coderoom_dir.join(ROLES_DIR).join("pm.md"), "pm priors\n").unwrap();
    fs::write(
        coderoom_dir.join(ROLES_DIR).join("backend.md"),
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
budget_per_role_usd = 0.5
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
}

#[test]
fn role_config_inherits_defaults() {
    let tmp = fixture(
        r#"
default_engine = "cc"
default_model = "opus"
budget_per_role_usd = 0.50
host_role = "pm"

[roles.pm]
[roles.backend]
"#,
    );
    let cfg = Config::load_test(tmp.path()).unwrap();
    let coderoom = tmp.path().join(CODEROOM_DIR);

    let pm = cfg.role_config("pm", &coderoom).unwrap();
    assert_eq!(pm.name, "pm");
    assert_eq!(pm.engine, Engine::Cc);
    assert_eq!(pm.model.as_deref(), Some("opus"));
    assert!((pm.budget_usd - 0.50).abs() < 1e-9);
    assert_eq!(pm.permission_mode, PermissionMode::Ask);

    let backend = cfg.role_config("backend", &coderoom).unwrap();
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
budget_per_role_usd = 0.50
host_role = "pm"

[roles.pm]
[roles.backend]

[roles.security]
engine = "codex"
model = "o3"
permission_mode = "bypass"
"#,
    );
    let coderoom = tmp.path().join(CODEROOM_DIR);
    // create the security priors so validation passes
    fs::write(
        coderoom.join(ROLES_DIR).join("security.md"),
        "security priors\n",
    )
    .unwrap();

    let cfg = Config::load_test(tmp.path()).unwrap();
    let security = cfg.role_config("security", &coderoom).unwrap();
    assert_eq!(security.engine, Engine::Codex);
    assert_eq!(security.model.as_deref(), Some("o3"));
    assert_eq!(security.permission_mode, PermissionMode::Bypass);
}

#[test]
fn codex_role_without_permission_override_uses_bypass() {
    let tmp = fixture(
        r#"
default_engine = "cc"
permission_mode = "ask"
budget_per_role_usd = 0.50
host_role = "pm"

[roles.pm]

[roles.security]
engine = "codex"
"#,
    );
    let coderoom = tmp.path().join(CODEROOM_DIR);
    fs::write(
        coderoom.join(ROLES_DIR).join("security.md"),
        "security priors\n",
    )
    .unwrap();

    let cfg = Config::load_test(tmp.path()).unwrap();
    let security = cfg.role_config("security", &coderoom).unwrap();
    assert_eq!(security.engine, Engine::Codex);
    assert_eq!(security.permission_mode, PermissionMode::Bypass);
}

#[test]
fn explicit_codex_permission_mode_is_preserved() {
    let tmp = fixture(
        r#"
default_engine = "cc"
permission_mode = "bypass"
budget_per_role_usd = 0.50
host_role = "pm"

[roles.pm]

[roles.security]
engine = "codex"
permission_mode = "ask"
"#,
    );
    let coderoom = tmp.path().join(CODEROOM_DIR);
    fs::write(
        coderoom.join(ROLES_DIR).join("security.md"),
        "security priors\n",
    )
    .unwrap();

    let cfg = Config::load_test(tmp.path()).unwrap();
    let security = cfg.role_config("security", &coderoom).unwrap();
    assert_eq!(security.engine, Engine::Codex);
    assert_eq!(security.permission_mode, PermissionMode::Ask);
}

#[test]
fn missing_host_role_is_rejected() {
    let tmp = fixture(
        r#"
default_engine = "cc"
budget_per_role_usd = 0.50
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
budget_per_role_usd = 0.50
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
fn invalid_budget_is_rejected() {
    let tmp = fixture(
        r#"
default_engine = "cc"
budget_per_role_usd = -1.0
host_role = "pm"

[roles.pm]
[roles.backend]
"#,
    );
    match Config::load_test(tmp.path()).expect_err("should reject negative budget") {
        ConfigError::InvalidBudget(b) => assert!((b - -1.0).abs() < 1e-9),
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn missing_config_file_surfaces_io_error() {
    let tmp = TempDir::new().unwrap();
    // don't even create .coderoom/
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
budget_per_role_usd = 0.50
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
