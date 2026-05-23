//! Integration coverage for priors liveness sidecars and doctor output.

use assert_cmd::Command;
use chrono::{Duration, SecondsFormat, Utc};
use predicates::prelude::*;
use std::fs;

use coreroom::config::COREROOM_DIR;
use coreroom::priors::ComposeOptions;

fn cr() -> Command {
    Command::cargo_bin("cr").expect("binary")
}

fn init_with_attached_knowledge() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project = tmp.path().to_str().expect("utf8 project");
    let source = tmp.path().join("runbook.md");
    fs::write(&source, "# Runbook\n\nKeep deployment notes current.\n").expect("source");

    cr().args(["init", "-y", "--project", project])
        .assert()
        .success();
    cr().args([
        "role",
        "attach",
        "engineer",
        source.to_str().expect("utf8 source"),
        "--project",
        project,
    ])
    .assert()
    .success();

    tmp
}

fn compose_engineer_with_liveness(tmp: &tempfile::TempDir) {
    coreroom::priors::compose_for_with_options(
        &tmp.path().join(COREROOM_DIR),
        "engineer",
        ComposeOptions {
            record_liveness: true,
            ..Default::default()
        },
    )
    .expect("compose with liveness");
}

#[test]
fn liveness_increments_when_role_is_composed() {
    let tmp = init_with_attached_knowledge();
    let coreroom = tmp.path().join(COREROOM_DIR);
    let segment_path = coreroom::liveness::knowledge_segment_path("engineer", "runbook.md");

    compose_engineer_with_liveness(&tmp);
    compose_engineer_with_liveness(&tmp);

    let doc = coreroom::liveness::read(&coreroom, "engineer").expect("liveness");
    let segment = doc
        .segments
        .get(&segment_path)
        .expect("knowledge liveness segment");
    assert_eq!(segment.hit_count, 2);
    assert!(segment.last_matched_at.is_some());
    assert!(!segment.attached_at.is_empty());
    assert!(coreroom::liveness::path_for_role(&coreroom, "engineer").is_file());
}

#[test]
fn role_knowledge_with_liveness_prints_hit_counts() {
    let tmp = init_with_attached_knowledge();
    compose_engineer_with_liveness(&tmp);

    cr().args([
        "role",
        "knowledge",
        "engineer",
        "--with-liveness",
        "--project",
        tmp.path().to_str().expect("utf8 project"),
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains("runbook.md"))
    .stdout(predicate::str::contains("hits"))
    .stdout(predicate::str::contains("last-loaded"));
}

#[test]
fn doctor_reports_stale_liveness_with_prune_command() {
    let tmp = init_with_attached_knowledge();
    let coreroom = tmp.path().join(COREROOM_DIR);
    compose_engineer_with_liveness(&tmp);

    let mut doc = coreroom::liveness::read(&coreroom, "engineer").expect("liveness");
    let old = (Utc::now() - Duration::days(31)).to_rfc3339_opts(SecondsFormat::Secs, true);
    let segment_path = coreroom::liveness::knowledge_segment_path("engineer", "runbook.md");
    let segment = doc
        .segments
        .get_mut(&segment_path)
        .expect("knowledge liveness segment");
    segment.last_matched_at = Some(old);
    segment.last_cited_at = None;
    coreroom::liveness::write(&coreroom, &doc).expect("write liveness");

    cr().args([
        "doctor",
        "--project",
        tmp.path().to_str().expect("utf8 project"),
        "--stale-days",
        "30",
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains("stale priors"))
    .stdout(predicate::str::contains(
        "cr role detach engineer runbook.md",
    ));
}
