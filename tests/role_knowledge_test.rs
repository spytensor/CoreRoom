//! Integration coverage for role knowledge CLI commands.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;

use coderoom::config::{CODEROOM_DIR, CONFIG_FILE, ROLES_DIR};

fn fixture() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    let coderoom = tmp.path().join(CODEROOM_DIR);
    fs::create_dir_all(coderoom.join(ROLES_DIR)).expect("roles dir");
    fs::write(
        coderoom.join(CONFIG_FILE),
        r#"
default_engine = "cc"
permission_mode = "ask"
host_role = "coral"

[roles.coral]
"#,
    )
    .expect("config");
    fs::write(coderoom.join(ROLES_DIR).join("coral.md"), "CORAL_PRIORS").expect("legacy priors");
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

    let coderoom = tmp.path().join(CODEROOM_DIR);
    assert!(!coderoom.join(ROLES_DIR).join("coral.md").exists());
    assert!(coderoom
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

    assert!(!coderoom
        .join(ROLES_DIR)
        .join("coral")
        .join("knowledge")
        .join("payload.md")
        .exists());
}
