//! Integration coverage for role knowledge CLI commands.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;

use coreroom::config::{CONFIG_FILE, COREROOM_DIR, ROLES_DIR};

fn fixture() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    let coreroom = tmp.path().join(COREROOM_DIR);
    fs::create_dir_all(coreroom.join(ROLES_DIR)).expect("roles dir");
    fs::write(
        coreroom.join(CONFIG_FILE),
        r#"
default_engine = "cc"
permission_mode = "ask"
host_role = "coral"

[roles.coral]
"#,
    )
    .expect("config");
    fs::write(coreroom.join(ROLES_DIR).join("coral.md"), "CORAL_PRIORS").expect("legacy priors");
    tmp
}

#[test]
fn role_knowledge_cli_attach_list_compose_and_detach() {
    let tmp = fixture();
    let source = tmp.path().join("coral-payload-v2.md");
    fs::write(&source, "CORAL_PAYLOAD_SPEC").expect("source");

    Command::cargo_bin("cr")
        .expect("binary")
        .args([
            "role",
            "attach",
            "coral",
            source.to_str().expect("utf8 path"),
            "--name",
            "payload.md",
            "--project",
            tmp.path().to_str().expect("utf8 project"),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("attached payload.md"))
        .stdout(predicate::str::contains("sha256:"));

    let coreroom = tmp.path().join(COREROOM_DIR);
    assert!(!coreroom.join(ROLES_DIR).join("coral.md").exists());
    assert!(coreroom
        .join(ROLES_DIR)
        .join("coral")
        .join("priors.md")
        .is_file());

    Command::cargo_bin("cr")
        .expect("binary")
        .args([
            "role",
            "knowledge",
            "coral",
            "--project",
            tmp.path().to_str().expect("utf8 project"),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("payload.md"))
        .stdout(predicate::str::contains("last-modified"));

    Command::cargo_bin("cr")
        .expect("binary")
        .args([
            "prompt",
            "show",
            "coral",
            "--project",
            tmp.path().to_str().expect("utf8 project"),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("CORAL_PAYLOAD_SPEC"));

    Command::cargo_bin("cr")
        .expect("binary")
        .args([
            "role",
            "detach",
            "coral",
            "payload.md",
            "--project",
            tmp.path().to_str().expect("utf8 project"),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("detached payload.md"));

    assert!(!coreroom
        .join(ROLES_DIR)
        .join("coral")
        .join("knowledge")
        .join("payload.md")
        .exists());
}
