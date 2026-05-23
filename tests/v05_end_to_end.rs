//! v0.5 virtual-team happy path across init, gate phases, audit events, and checks.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};

use coderoom::config::CODEROOM_DIR;

const THREAD: &str = "v05-e2e";

fn cr() -> Command {
    Command::cargo_bin("cr").expect("binary")
}

fn project_arg(project: &Path) -> &str {
    project.to_str().expect("utf8 project")
}

fn gate_dir(project: &Path) -> PathBuf {
    project.join(CODEROOM_DIR).join("gates").join(THREAD)
}

fn write_gate_file(project: &Path, name: &str, body: &str) -> String {
    let path = gate_dir(project).join(name);
    fs::write(&path, body).expect("write gate artifact");
    format!(".coderoom/gates/{THREAD}/{name}")
}

fn assert_success(args: &[&str]) {
    cr().args(args).assert().success();
}

#[test]
fn v05_virtual_team_gate_happy_path() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project = project_arg(tmp.path());
    setup_project(tmp.path(), project);
    start_gate(project);
    record_discovery(tmp.path(), project);
    record_plan(tmp.path(), project);
    record_review(tmp.path(), project);
    record_signoff(tmp.path(), project);
    record_qa_and_close(project);
    assert_final_state(tmp.path(), project);
}

fn setup_project(root: &Path, project: &str) {
    fs::create_dir_all(root.join("src")).expect("src dir");
    fs::write(
        root.join("src").join("main.rs"),
        "fn main() {\n    println!(\"v05\");\n}\n",
    )
    .expect("source");

    assert_success(&["init", "-y", "--preset", "team", "--project", project]);
    assert_success(&[
        "config",
        "set",
        "default_model",
        "claude-sonnet-4",
        "--project-layer",
        "--project",
        project,
    ]);
    assert_success(&[
        "role",
        "set-authority",
        "sre",
        "infra",
        "--project",
        project,
    ]);
}

fn start_gate(project: &str) {
    assert_success(&[
        "gate",
        "init",
        "--thread",
        THREAD,
        "--tier",
        "1",
        "--feature",
        "v0.5 end-to-end happy path",
        "--project",
        project,
    ]);
    assert_success(&[
        "gate",
        "implementer",
        "--thread",
        THREAD,
        "--role",
        "engineer",
        "--engine",
        "cc",
        "--model",
        "claude-sonnet-4",
        "--turn",
        "tu-impl",
        "--project",
        project,
    ]);
}

fn record_discovery(root: &Path, project: &str) {
    cr().args([
        "gate",
        "phase",
        THREAD,
        "discovery",
        "--actor",
        "host",
        "--project",
        project,
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains(
        "advanced v05-e2e: intake -> discovery",
    ));
    let discovery = write_gate_file(
        root,
        "discovery.md",
        "# Discovery\n\nScope verified against src/main.rs:1.\n",
    );
    assert_success(&[
        "gate",
        "artifact",
        "--thread",
        THREAD,
        "--kind",
        "discovery",
        "--path",
        &discovery,
        "--role",
        "host",
        "--project",
        project,
    ]);
}

fn record_plan(root: &Path, project: &str) {
    assert_success(&[
        "gate",
        "phase",
        THREAD,
        "plan",
        "--actor",
        "host",
        "--project",
        project,
    ]);
    let plan = write_gate_file(
        root,
        "plan.md",
        "---\nscopes: [infra]\n---\n\n# Plan\n\nUse the existing CLI gate flow.\n\n## Sign-off Checklist\n\n| ID | Owner | Check | Evidence |\n| - | - | - | - |\n| SO-1 | host | CLI happy path is verified | pending |\n",
    );
    assert_success(&[
        "gate",
        "artifact",
        "--thread",
        THREAD,
        "--kind",
        "plan",
        "--path",
        &plan,
        "--role",
        "host",
        "--project",
        project,
    ]);
}

fn record_review(root: &Path, project: &str) {
    assert_success(&[
        "gate",
        "phase",
        THREAD,
        "review",
        "--actor",
        "host",
        "--project",
        project,
    ]);
    assert_success(&[
        "gate",
        "role-review",
        THREAD,
        "sre",
        "approve",
        "--project",
        project,
    ]);
    let review = write_gate_file(
        root,
        "review.md",
        "reviewer_role: reviewer\nengine: codex\nmodel: gpt-5\nblocking_count: 0\nwarning_count: 0\ncross_model_satisfied: true\nall_blockings_resolved: true\n\nEvidence: src/main.rs:1 covers the executable entrypoint.\n",
    );
    for role in ["reviewer", "qa"] {
        assert_success(&[
            "gate",
            "reviewer",
            "--thread",
            THREAD,
            "--role",
            role,
            "--engine",
            "codex",
            "--model",
            "gpt-5",
            "--artifact",
            &review,
            "--file-line-evidence",
            "--all-blockings-resolved",
            "--project",
            project,
        ]);
    }
}

fn record_signoff(root: &Path, project: &str) {
    assert_success(&[
        "gate",
        "phase",
        THREAD,
        "signoff",
        "--actor",
        "host",
        "--project",
        project,
    ]);
    let signoff = write_gate_file(
        root,
        "signoff.md",
        "# Signoff\n\nSO-1: cargo test and gate validation recorded in this thread.\n",
    );
    assert_success(&[
        "gate",
        "artifact",
        "--thread",
        THREAD,
        "--kind",
        "signoff",
        "--path",
        &signoff,
        "--role",
        "host",
        "--project",
        project,
    ]);
}

fn record_qa_and_close(project: &str) {
    assert_success(&[
        "gate",
        "phase",
        THREAD,
        "implement",
        "--actor",
        "engineer",
        "--project",
        project,
    ]);
    assert_success(&[
        "gate",
        "phase",
        THREAD,
        "qa",
        "--actor",
        "qa",
        "--project",
        project,
    ]);
    assert_success(&[
        "gate",
        "verify",
        "--thread",
        THREAD,
        "--command",
        "cargo test",
        "--evidence",
        "cargo test passed for the v0.5 end-to-end fixture",
        "--ok",
        "--project",
        project,
    ]);
    cr().args(["gate", "validate", "--thread", THREAD, "--project", project])
        .assert()
        .success()
        .stdout(predicate::str::contains("Tier 1 gate pass"));
    assert_success(&[
        "gate",
        "phase",
        THREAD,
        "closed",
        "--actor",
        "host",
        "--project",
        project,
    ]);
    assert_success(&["gate", "close", "--thread", THREAD, "--project", project]);
}

fn assert_final_state(root: &Path, project: &str) {
    cr().args(["gate", "status", "--thread", THREAD, "--project", project])
        .assert()
        .success()
        .stdout(predicate::str::contains("closed"))
        .stdout(predicate::str::contains("pass"));
    cr().args(["verify", "--project", project])
        .assert()
        .success()
        .stdout(predicate::str::contains("priors lock verified"));
    cr().args(["doctor", "--project", project])
        .assert()
        .success()
        .stdout(predicate::str::contains("no stale priors"));

    let log =
        fs::read_to_string(root.join(CODEROOM_DIR).join("messages.jsonl")).expect("messages log");
    assert!(log.contains(r#""type":"phase_advanced""#));
    assert!(log.contains(r#""from":"intake""#));
    assert!(log.contains(r#""to":"closed""#));
    assert!(log.contains(r#""type":"plan_reviewed""#));
    assert!(log.contains(r#""role":"sre""#));
}
