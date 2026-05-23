//! Integration coverage for `cr lock` and `cr verify`.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;

use coreroom::config::COREROOM_DIR;

fn cr() -> Command {
    Command::cargo_bin("cr").expect("binary")
}

#[test]
fn init_writes_lock_and_verify_passes() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project = tmp.path().to_str().expect("utf8 project");

    cr().args(["init", "-y", "--project", project])
        .assert()
        .success();

    let lock = coreroom::lock::read(&tmp.path().join(COREROOM_DIR)).expect("lock parses");
    assert!(lock.roles.contains_key("host"));
    assert!(lock.roles["host"]
        .layers
        .iter()
        .any(|layer| layer.kind == "kernel"));
    assert!(lock.roles["host"]
        .layers
        .iter()
        .any(|layer| layer.kind == "role"));

    cr().args(["verify", "--project", project])
        .assert()
        .success()
        .stdout(predicate::str::contains("priors lock verified"));
}

#[test]
fn verify_reports_drift_and_lock_regenerates() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project = tmp.path().to_str().expect("utf8 project");
    cr().args(["init", "-y", "--project", project])
        .assert()
        .success();

    fs::write(
        tmp.path()
            .join(COREROOM_DIR)
            .join("roles")
            .join("host")
            .join("priors.md"),
        "tampered host priors\n",
    )
    .expect("tamper priors");

    cr().args(["verify", "--project", project])
        .assert()
        .failure()
        .stdout(predicate::str::contains("role @host").and(predicate::str::contains("drift")));

    cr().args(["lock", "--project", project])
        .assert()
        .success()
        .stdout(predicate::str::contains("wrote"));
    cr().args(["verify", "--project", project])
        .assert()
        .success();
}

#[test]
fn lock_records_mounted_knowledge_layers() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project = tmp.path().to_str().expect("utf8 project");
    let knowledge = tmp.path().join("runbook.md");
    fs::write(&knowledge, "# Runbook\n\nKeep deploys boring.\n").expect("knowledge");

    cr().args(["init", "-y", "--project", project])
        .assert()
        .success();
    cr().args([
        "role",
        "attach",
        "engineer",
        knowledge.to_str().expect("utf8 knowledge"),
        "--project",
        project,
    ])
    .assert()
    .success();
    cr().args(["lock", "--project", project]).assert().success();

    let lock = coreroom::lock::read(&tmp.path().join(COREROOM_DIR)).expect("lock parses");
    assert!(lock.roles["engineer"].layers.iter().any(|layer| {
        layer.kind == "knowledge" && layer.path.ends_with("/knowledge/runbook.md")
    }));
}
