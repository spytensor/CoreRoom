//! Live local snapshot builder for the CoreRoom console.
//!
//! The v0.9.0 console could only render prebuilt snapshot files. This module
//! builds a conservative snapshot from real local project facts so the normal
//! user path can enter the console without a fixture.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

use crate::config::{Config, COREROOM_DIR};
use crate::console_snapshot::{
    ConversationSnapshot, ConversationTurn, ConversationVisibility, CoreRoomSnapshot, DirtyState,
    EvidenceClosureState, EvidenceSnapshot, GateSnapshot, GitHubSnapshot, LayoutHints,
    ProjectIdentity, RoleLaneState, RoleMaterialFreshness, RoleRuntimeSnapshot, SessionFreshness,
    SourceHealthSnapshot, SourceHealthState, StatusState, WorkLifecycle, WorkSnapshot,
};

/// Build a console snapshot from the current local project state.
pub fn snapshot_from_project(project_root: &Path) -> Result<CoreRoomSnapshot> {
    let cfg = Config::load(project_root)
        .with_context(|| format!("loading {}", project_root.join(COREROOM_DIR).display()))?;
    let git = GitFacts::observe(project_root);
    let repository = git
        .repository
        .clone()
        .unwrap_or_else(|| project_root_name(project_root));
    let tracker_issue = default_tracker_issue(&repository);
    let roles = role_snapshots(project_root, &cfg)?;
    let visible_roles = roles
        .iter()
        .map(|role| role.role.clone())
        .collect::<Vec<_>>();

    let snapshot = CoreRoomSnapshot {
        schema_version: crate::console_snapshot::CONSOLE_SNAPSHOT_SCHEMA_VERSION,
        project: ProjectIdentity {
            project: project_root_name(project_root),
            repository: repository.clone(),
            remote: git.remote.clone(),
            branch: git.branch.clone().unwrap_or_else(|| "unknown".to_owned()),
            head_sha: git.head_sha.clone(),
            dirty_state: git.dirty_state,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            active_phase: "local room".to_owned(),
            tracker_issue,
        },
        runtime: crate::console_snapshot::RuntimeSnapshot {
            room_id: Some("local-console".to_owned()),
            host_role: cfg.host_role.clone(),
            session_state: SessionFreshness::Unknown,
            permission_mode: Some(cfg.permission_mode.as_str().to_owned()),
            roles,
            active_role: Some(cfg.host_role.clone()),
            waiting_approval: false,
        },
        conversation: ConversationSnapshot {
            public_turns: vec![
                ConversationTurn {
                    speaker: "user".to_owned(),
                    body: "Opened CoreRoom.".to_owned(),
                    visibility: ConversationVisibility::PublicTranscript,
                },
                ConversationTurn {
                    speaker: cfg.host_role.clone(),
                    body: "Console-first room loaded from local project facts. Exit the console to continue in the REPL, or run `cr start` to skip this dashboard.".to_owned(),
                    visibility: ConversationVisibility::PublicTranscript,
                },
            ],
            internal_delegation_count: 0,
            internal_activity: Vec::new(),
        },
        work: vec![WorkSnapshot {
            id: "WO-0000".to_owned(),
            title: "Live CoreRoom session".to_owned(),
            phase: Some("local room".to_owned()),
            epic: Some("console-entrypoint".to_owned()),
            github_issue: None,
            branch: git.branch.clone(),
            pull_request: None,
            ci_state: StatusState::Unknown,
            evidence_state: StatusState::Warn,
            tracker_state: StatusState::Warn,
            lifecycle: WorkLifecycle::Ready,
            source_citations: vec!["local:.coreroom/config.toml".to_owned(), "git:HEAD".to_owned()],
        }],
        gates: vec![GateSnapshot {
            work_order: "WO-0000".to_owned(),
            current_phase: "intake".to_owned(),
            blocked_reason: None,
            missing_reviews: Vec::new(),
            stale_plan_sha: None,
            signoff_ready: false,
        }],
        evidence: vec![EvidenceSnapshot {
            work_order: "WO-0000".to_owned(),
            status: EvidenceClosureState::Incomplete,
            missing_fields: vec!["github issue".to_owned(), "PR evidence".to_owned()],
            unverified_items: vec!["live GitHub counts are not fetched by the local console".to_owned()],
            rollback: Some("Exit the console; no project state is mutated by this view.".to_owned()),
            tracker_updated: false,
        }],
        sources: vec![SourceHealthSnapshot {
            source_id: "local-project".to_owned(),
            status: match git.head_sha {
                Some(_) => SourceHealthState::Pinned,
                None => SourceHealthState::Missing,
            },
            pin: git.head_sha,
            trust_level: "project-source".to_owned(),
            visible_roles,
            findings: git.findings,
            related_work_orders: vec!["WO-0000".to_owned()],
        }],
        github: GitHubSnapshot {
            repository,
            tracker_issue,
            open_issues: 0,
            open_pull_requests: 0,
            failing_checks: 0,
        },
        alerts: Vec::new(),
        layout: LayoutHints {
            primary_pane: "public-conversation".to_owned(),
            min_columns: 80,
            preferred_columns: 160,
            collapsed_panes: vec!["right-rail".to_owned()],
        },
    };
    snapshot.validate()?;
    Ok(snapshot)
}

fn role_snapshots(project_root: &Path, cfg: &Config) -> Result<Vec<RoleRuntimeSnapshot>> {
    let coreroom_dir = project_root.join(COREROOM_DIR);
    let mut names = cfg.role_names().map(str::to_owned).collect::<Vec<_>>();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let role = cfg
                .role_config(&name, &coreroom_dir)
                .with_context(|| format!("resolving role `{name}`"))?;
            let is_host = cfg.is_host(&name);
            Ok(RoleRuntimeSnapshot {
                role: name,
                enabled: true,
                engine: role.engine.as_str().to_owned(),
                model: role.model,
                permission_mode: Some(role.permission_mode.as_str().to_owned()),
                session_state: SessionFreshness::Unknown,
                priors_freshness: Some(RoleMaterialFreshness::Unknown),
                knowledge_freshness: Some(RoleMaterialFreshness::Unknown),
                state: if is_host {
                    RoleLaneState::Idle
                } else {
                    RoleLaneState::Enabled
                },
                waiting_approval: false,
                current_work_order: is_host.then(|| "WO-0000".to_owned()),
                current_gate_phase: is_host.then(|| "intake".to_owned()),
                last_activity: Some(if is_host {
                    "Ready to receive user intent".to_owned()
                } else {
                    "Configured and available".to_owned()
                }),
            })
        })
        .collect()
}

fn project_root_name(project_root: &Path) -> String {
    project_root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("CoreRoom Project")
        .to_owned()
}

fn default_tracker_issue(repository: &str) -> u64 {
    if repository.eq_ignore_ascii_case("spytensor/CoreRoom") {
        295
    } else {
        1
    }
}

#[derive(Debug)]
struct GitFacts {
    remote: Option<String>,
    repository: Option<String>,
    branch: Option<String>,
    head_sha: Option<String>,
    dirty_state: DirtyState,
    findings: Vec<String>,
}

impl GitFacts {
    fn observe(project_root: &Path) -> Self {
        let remote = git_output(project_root, ["config", "--get", "remote.origin.url"]);
        let branch = git_output(project_root, ["branch", "--show-current"]);
        let head_sha = git_output(project_root, ["rev-parse", "--short", "HEAD"]);
        let status = git_output_allow_empty(project_root, ["status", "--porcelain"]);
        let dirty_state = match status {
            Some(ref output) if output.is_empty() => DirtyState::Clean,
            Some(_) => DirtyState::Dirty,
            None => DirtyState::Unknown,
        };
        let mut findings = Vec::new();
        if matches!(dirty_state, DirtyState::Dirty) {
            findings.push("worktree has local changes".to_owned());
        }
        if remote.is_none() {
            findings.push("remote origin not configured".to_owned());
        }
        Self {
            repository: remote.as_deref().and_then(repository_from_remote),
            remote,
            branch,
            head_sha,
            dirty_state,
            findings,
        }
    }
}

fn git_output<const N: usize>(project_root: &Path, args: [&str; N]) -> Option<String> {
    git_output_inner(project_root, args, false)
}

fn git_output_allow_empty<const N: usize>(project_root: &Path, args: [&str; N]) -> Option<String> {
    git_output_inner(project_root, args, true)
}

fn git_output_inner<const N: usize>(
    project_root: &Path,
    args: [&str; N],
    allow_empty: bool,
) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(project_root)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (allow_empty || !value.is_empty()).then_some(value)
}

fn repository_from_remote(remote: &str) -> Option<String> {
    let trimmed = remote.trim_end_matches(".git");
    if let Some(rest) = trimmed.strip_prefix("git@github.com:") {
        return owner_repo(rest);
    }
    if let Some(rest) = trimmed.strip_prefix("https://github.com/") {
        return owner_repo(rest);
    }
    if let Some(rest) = trimmed.strip_prefix("http://github.com/") {
        return owner_repo(rest);
    }
    None
}

fn owner_repo(value: &str) -> Option<String> {
    let mut parts = value.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn builds_live_snapshot_from_project_config_and_git() {
        let tmp = tempfile::tempdir().unwrap();
        write_project(tmp.path());
        run_git(tmp.path(), ["init"]);
        run_git(tmp.path(), ["config", "user.email", "test@example.com"]);
        run_git(tmp.path(), ["config", "user.name", "Test User"]);
        run_git(
            tmp.path(),
            [
                "remote",
                "add",
                "origin",
                "git@github.com:spytensor/CoreRoom.git",
            ],
        );
        run_git(tmp.path(), ["add", "."]);
        run_git(tmp.path(), ["commit", "-m", "init"]);

        let snapshot = snapshot_from_project(tmp.path()).unwrap();

        assert_eq!(snapshot.project.repository, "spytensor/CoreRoom");
        assert_eq!(snapshot.project.tracker_issue, 295);
        assert_eq!(snapshot.project.dirty_state, DirtyState::Clean);
        assert_eq!(snapshot.runtime.host_role, "host");
        assert_eq!(snapshot.runtime.roles.len(), 2);
        assert_eq!(snapshot.work[0].id, "WO-0000");
        assert!(snapshot.conversation.public_turns[1]
            .body
            .contains("Console-first room"));
    }

    #[test]
    fn parses_common_github_remote_forms() {
        assert_eq!(
            repository_from_remote("git@github.com:spytensor/CoreRoom.git").as_deref(),
            Some("spytensor/CoreRoom")
        );
        assert_eq!(
            repository_from_remote("https://github.com/spytensor/CoreRoom.git").as_deref(),
            Some("spytensor/CoreRoom")
        );
    }

    fn write_project(root: &Path) {
        let coreroom = root.join(COREROOM_DIR);
        fs::create_dir_all(coreroom.join("roles/host")).unwrap();
        fs::create_dir_all(coreroom.join("roles/reviewer")).unwrap();
        fs::write(
            coreroom.join("config.toml"),
            r#"
default_engine = "cc"
permission_mode = "ask"
host_role = "host"

[roles.host]

[roles.reviewer]
engine = "codex"
permission_mode = "bypass"
"#,
        )
        .unwrap();
        fs::write(coreroom.join("roles/host/priors.md"), "host").unwrap();
        fs::write(coreroom.join("roles/reviewer/priors.md"), "reviewer").unwrap();
    }

    fn run_git<const N: usize>(root: &Path, args: [&str; N]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
