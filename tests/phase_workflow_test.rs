//! Integration coverage for explicit gate phase workflow commands.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;

use coderoom::config::CODEROOM_DIR;

#[test]
fn gate_phase_cli_advances_creates_artifact_and_rejects_skip() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project = tmp.path().to_str().expect("utf8 project");

    Command::cargo_bin("cr")
        .expect("binary")
        .args([
            "gate",
            "init",
            "--thread",
            "42",
            "--tier",
            "1",
            "--feature",
            "phase flow",
            "--project",
            project,
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("created Tier 1 gate"));

    assert!(tmp
        .path()
        .join(CODEROOM_DIR)
        .join("gates")
        .join("42")
        .join("intake.md")
        .is_file());

    Command::cargo_bin("cr")
        .expect("binary")
        .args([
            "gate",
            "phase",
            "42",
            "discovery",
            "--actor",
            "user",
            "--project",
            project,
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("advanced 42: intake -> discovery"));

    assert!(tmp
        .path()
        .join(CODEROOM_DIR)
        .join("gates")
        .join("42")
        .join("discovery.md")
        .is_file());
    let log = fs::read_to_string(tmp.path().join(CODEROOM_DIR).join("messages.jsonl"))
        .expect("messages log");
    assert!(log.contains(r#""type":"phase_advanced""#));
    assert!(log.contains(r#""from":"intake""#));
    assert!(log.contains(r#""to":"discovery""#));

    Command::cargo_bin("cr")
        .expect("binary")
        .args(["gate", "phase", "42", "closed", "--project", project])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot advance"));
}
