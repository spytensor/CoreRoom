//! Integration coverage for authority-scoped plan review and override commands.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;

use coreroom::config::COREROOM_DIR;

fn write_fixture_config(project: &Path) {
    let coreroom = project.join(COREROOM_DIR);
    fs::create_dir_all(coreroom.join("roles")).expect("roles dir");
    fs::write(
        coreroom.join("config.toml"),
        r#"
default_engine = "cc"
default_model = "claude-sonnet-4"
host_role = "pm"

[roles.pm]

[roles.sre]
engine = "codex"
model = "gpt-5"
authority = ["infra"]

[roles.release]
engine = "cc"
model = "claude-opus-4"
authority = ["deployment"]

[roles.security]
engine = "codex"
model = "gpt-5"
authority = ["secrets"]
"#,
    )
    .expect("config");
    for role in ["pm", "sre", "release", "security"] {
        fs::write(
            coreroom.join("roles").join(format!("{role}.md")),
            "priors\n",
        )
        .expect("priors");
    }
}

fn write_plan(project: &Path, thread: &str, scopes: &str) {
    let path = project
        .join(COREROOM_DIR)
        .join("gates")
        .join(thread)
        .join("plan.md");
    fs::write(
        path,
        format!(
            "---\nscopes: [{scopes}]\n---\n\n# Plan\n\n## Sign-off Checklist\n\n| ID | Owner | Check | Evidence |\n| - | - | - | - |\n| SO-1 | host | Ready | TBD |\n"
        ),
    )
    .expect("plan");
}

#[test]
fn plan_signoff_happy_path_requires_all_matching_authority_roles() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_fixture_config(tmp.path());
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
            "plan signoff",
            "--phase",
            "plan",
            "--project",
            project,
        ])
        .assert()
        .success();
    write_plan(tmp.path(), "42", "infra, deployment");

    Command::cargo_bin("cr")
        .expect("binary")
        .args(["gate", "phase", "42", "review", "--project", project])
        .assert()
        .success();
    Command::cargo_bin("cr")
        .expect("binary")
        .args(["gate", "phase", "42", "signoff", "--project", project])
        .assert()
        .failure()
        .stderr(predicate::str::contains("@sre"));

    for role in ["sre", "release"] {
        Command::cargo_bin("cr")
            .expect("binary")
            .args([
                "gate",
                "role-review",
                "42",
                role,
                "approve",
                "--project",
                project,
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("recorded approve review"));
    }

    Command::cargo_bin("cr")
        .expect("binary")
        .args(["gate", "status", "--thread", "42", "--project", project])
        .assert()
        .success()
        .stdout(predicate::str::contains("@sre [infra]: approve"))
        .stdout(predicate::str::contains("@release [deployment]: approve"));
    Command::cargo_bin("cr")
        .expect("binary")
        .args(["gate", "phase", "42", "signoff", "--project", project])
        .assert()
        .success()
        .stdout(predicate::str::contains("advanced 42: review -> signoff"));

    assert!(tmp
        .path()
        .join(COREROOM_DIR)
        .join("gates")
        .join("42")
        .join("reviews")
        .join("sre.toml")
        .is_file());
    let log =
        fs::read_to_string(tmp.path().join(COREROOM_DIR).join("messages.jsonl")).expect("log");
    assert!(log.contains(r#""type":"plan_reviewed""#));
}

#[test]
fn plan_signoff_reject_then_override_path() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_fixture_config(tmp.path());
    let project = tmp.path().to_str().expect("utf8 project");

    Command::cargo_bin("cr")
        .expect("binary")
        .args([
            "gate",
            "init",
            "--thread",
            "43",
            "--tier",
            "1",
            "--feature",
            "secret rotation",
            "--phase",
            "plan",
            "--project",
            project,
        ])
        .assert()
        .success();
    write_plan(tmp.path(), "43", "secrets");
    Command::cargo_bin("cr")
        .expect("binary")
        .args(["gate", "phase", "43", "review", "--project", project])
        .assert()
        .success();

    Command::cargo_bin("cr")
        .expect("binary")
        .args([
            "gate",
            "role-review",
            "43",
            "security",
            "reject",
            "--reason",
            "missing rotation rollback",
            "--project",
            project,
        ])
        .assert()
        .success();
    Command::cargo_bin("cr")
        .expect("binary")
        .args(["gate", "phase", "43", "signoff", "--project", project])
        .assert()
        .failure()
        .stderr(predicate::str::contains("@security"));

    Command::cargo_bin("cr")
        .expect("binary")
        .args([
            "gate",
            "override",
            "43",
            "--role",
            "security",
            "--reason",
            "accepted for emergency patch",
            "--project",
            project,
        ])
        .assert()
        .success();
    Command::cargo_bin("cr")
        .expect("binary")
        .args(["gate", "phase", "43", "signoff", "--project", project])
        .assert()
        .success()
        .stdout(predicate::str::contains("advanced 43: review -> signoff"));

    let log =
        fs::read_to_string(tmp.path().join(COREROOM_DIR).join("messages.jsonl")).expect("log");
    assert!(log.contains(r#""type":"plan_overridden""#));
}
