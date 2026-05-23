//! Integration coverage for `cr init` hook scaffolding.

use assert_cmd::Command;
use std::fs;
use std::path::{Path, PathBuf};

use coderoom::config::{CODEROOM_DIR, CONFIG_FILE, ROLES_DIR};

fn cr() -> Command {
    Command::cargo_bin("cr").expect("binary")
}

fn settings_json(project: &Path) -> serde_json::Value {
    let text =
        fs::read_to_string(project.join(".claude").join("settings.json")).expect("settings.json");
    serde_json::from_str(&text).expect("valid settings json")
}

fn backup_files(project: &Path) -> Vec<PathBuf> {
    let claude = project.join(".claude");
    if !claude.is_dir() {
        return Vec::new();
    }
    let mut files = fs::read_dir(claude)
        .expect("claude dir")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("settings.json.bak."))
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}

#[test]
fn init_with_claude_hooks_team_preset_is_idempotent() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project = tmp.path().to_str().expect("utf8 project");

    cr().args([
        "init",
        "-y",
        "--with-claude-hooks",
        "--preset",
        "team",
        "--project",
        project,
    ])
    .assert()
    .success();

    for role in ["host", "engineer", "reviewer", "sre", "security", "qa"] {
        assert!(tmp
            .path()
            .join(CODEROOM_DIR)
            .join(ROLES_DIR)
            .join(role)
            .join("priors.md")
            .is_file());
    }
    let settings = settings_json(tmp.path());
    assert!(settings.to_string().contains("__coderoom-hook-decision"));
    assert!(tmp
        .path()
        .join(".claude")
        .join(".coderoom-managed.json")
        .is_file());

    let config_before =
        fs::read_to_string(tmp.path().join(CODEROOM_DIR).join(CONFIG_FILE)).expect("config");
    let settings_before =
        fs::read_to_string(tmp.path().join(".claude").join("settings.json")).expect("settings");
    cr().args([
        "init",
        "-y",
        "--with-claude-hooks",
        "--preset",
        "team",
        "--project",
        project,
    ])
    .assert()
    .success();

    assert_eq!(
        config_before,
        fs::read_to_string(tmp.path().join(CODEROOM_DIR).join(CONFIG_FILE)).expect("config")
    );
    assert_eq!(
        settings_before,
        fs::read_to_string(tmp.path().join(".claude").join("settings.json")).expect("settings")
    );
    assert!(backup_files(tmp.path()).is_empty());
}

#[test]
fn init_merges_existing_claude_settings_and_creates_backup() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let claude = tmp.path().join(".claude");
    fs::create_dir_all(&claude).expect("claude dir");
    fs::write(
        claude.join("settings.json"),
        r#"{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          { "type": "command", "command": "echo keep-existing-hook" }
        ]
      }
    ]
  }
}
"#,
    )
    .expect("existing settings");
    let project = tmp.path().to_str().expect("utf8 project");

    cr().args(["init", "-y", "--with-claude-hooks", "--project", project])
        .assert()
        .success();

    let rendered = settings_json(tmp.path()).to_string();
    assert!(rendered.contains("echo keep-existing-hook"));
    assert!(rendered.contains("__coderoom-hook-decision"));
    assert_eq!(backup_files(tmp.path()).len(), 1);
}

#[test]
fn init_with_existing_coderoom_can_still_upgrade_hooks() {
    let tmp = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(tmp.path().join(CODEROOM_DIR)).expect("coderoom dir");
    let project = tmp.path().to_str().expect("utf8 project");

    cr().args(["init", "--upgrade-hooks", "--project", project])
        .assert()
        .success();

    assert!(tmp.path().join(CODEROOM_DIR).is_dir());
    assert!(tmp.path().join(".claude").join("settings.json").is_file());
}
