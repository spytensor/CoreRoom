use super::*;
use crate::config::ROLES_DIR;
use std::fs;
use tempfile::TempDir;

fn fixture() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let coreroom = tmp.path().join(COREROOM_DIR);
    fs::create_dir_all(coreroom.join(ROLES_DIR)).unwrap();
    fs::write(
        coreroom.join(CONFIG_FILE),
        "default_engine = \"cc\"\n\
         host_role = \"host\"\n\n\
         [roles.host]\n",
    )
    .unwrap();
    fs::write(coreroom.join(ROLES_DIR).join("host.md"), "h").unwrap();
    tmp
}

#[test]
fn resolve_paths_for_each_layer() {
    let tmp = fixture();
    let root = tmp.path();
    let project = resolve_path(LayerTarget::Project, root).unwrap();
    assert_eq!(project, root.join(COREROOM_DIR).join(CONFIG_FILE));
    let local = resolve_path(LayerTarget::Local, root).unwrap();
    assert_eq!(local, root.join(COREROOM_DIR).join(CONFIG_LOCAL_FILE));
}

#[test]
fn ensure_seeded_creates_file_with_stub() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("a/b/c.toml");
    ensure_seeded(&target, "stub-content\n").unwrap();
    assert!(target.is_file());
    assert_eq!(fs::read_to_string(&target).unwrap(), "stub-content\n");
}

#[test]
fn ensure_seeded_is_no_op_when_file_exists() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("c.toml");
    fs::write(&target, "user-content").unwrap();
    ensure_seeded(&target, "stub").unwrap();
    assert_eq!(fs::read_to_string(&target).unwrap(), "user-content");
}

#[test]
fn ensure_local_gitignored_creates_when_missing() {
    let tmp = TempDir::new().unwrap();
    let coreroom = tmp.path().join(COREROOM_DIR);
    fs::create_dir_all(&coreroom).unwrap();
    ensure_local_gitignored(&coreroom).unwrap();
    let body = fs::read_to_string(coreroom.join(".gitignore")).unwrap();
    assert!(body.contains(CONFIG_LOCAL_FILE));
}

#[test]
fn ensure_local_gitignored_appends_to_existing() {
    let tmp = TempDir::new().unwrap();
    let coreroom = tmp.path().join(COREROOM_DIR);
    fs::create_dir_all(&coreroom).unwrap();
    fs::write(coreroom.join(".gitignore"), "patches/\n").unwrap();
    ensure_local_gitignored(&coreroom).unwrap();
    let body = fs::read_to_string(coreroom.join(".gitignore")).unwrap();
    assert!(body.contains("patches/"));
    assert!(body.contains(CONFIG_LOCAL_FILE));
}

#[test]
fn ensure_local_gitignored_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    let coreroom = tmp.path().join(COREROOM_DIR);
    fs::create_dir_all(&coreroom).unwrap();
    ensure_local_gitignored(&coreroom).unwrap();
    ensure_local_gitignored(&coreroom).unwrap();
    let body = fs::read_to_string(coreroom.join(".gitignore")).unwrap();
    // Only one occurrence of the rule, even after two calls.
    assert_eq!(
        body.matches(CONFIG_LOCAL_FILE).count(),
        1,
        "gitignore body: {body}"
    );
}

#[test]
fn pick_editor_prefers_visual_over_editor() {
    let env = |k: &str| match k {
        "VISUAL" => Some("/usr/bin/vim".to_owned()),
        "EDITOR" => Some("/usr/bin/nano".to_owned()),
        _ => None,
    };
    assert_eq!(
        pick_editor_from(env).unwrap(),
        PathBuf::from("/usr/bin/vim")
    );
}

#[test]
fn pick_editor_falls_back_to_editor_when_visual_empty() {
    let env = |k: &str| match k {
        "VISUAL" => Some(String::new()),
        "EDITOR" => Some("/usr/bin/nano".to_owned()),
        _ => None,
    };
    assert_eq!(
        pick_editor_from(env).unwrap(),
        PathBuf::from("/usr/bin/nano")
    );
}

#[test]
fn pick_editor_errors_when_unset() {
    let env = |_: &str| None;
    let err = pick_editor_from(env).expect_err("no editor");
    assert!(err.to_string().contains("EDITOR"));
}

#[test]
fn show_runs_against_a_minimal_project() {
    let tmp = fixture();
    // Smoke test — show writes to stdout; we just verify it
    // doesn't error against a valid project. Use `show_with_user`
    // with `None` so the developer's real config doesn't leak in.
    show_with_user(tmp.path(), None).unwrap();
}

#[test]
fn path_subcommand_prints_each_layer() {
    let tmp = fixture();
    path(LayerTarget::Project, tmp.path()).unwrap();
    path(LayerTarget::Local, tmp.path()).unwrap();
    // user can be None in some CI envs; tolerate either.
    let _ = path(LayerTarget::User, tmp.path());
}

#[test]
fn set_project_scalar_updates_config() {
    let tmp = fixture();
    set(LayerTarget::Project, tmp.path(), "default_model", "opus").unwrap();
    let cfg = crate::config::Config::load_test(tmp.path()).unwrap();
    assert_eq!(cfg.default_model.as_deref(), Some("opus"));
}

#[test]
fn edit_project_refuses_when_config_missing() {
    let tmp = TempDir::new().unwrap();
    // No .coreroom/config.toml
    let err = edit(LayerTarget::Project, tmp.path()).expect_err("should refuse");
    assert!(err.to_string().contains("cr init"));
}

#[test]
fn edit_local_refuses_when_coreroom_dir_missing() {
    let tmp = TempDir::new().unwrap();
    let err = edit(LayerTarget::Local, tmp.path()).expect_err("should refuse");
    assert!(err.to_string().contains("cr init"));
}
