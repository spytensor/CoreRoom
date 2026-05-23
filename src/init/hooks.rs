use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

const CLAUDE_DIR: &str = ".claude";
const SETTINGS_FILE: &str = "settings.json";
const MARKER_FILE: &str = ".coderoom-managed.json";
const HOOK_COMMAND: &str =
    "cr __coderoom-hook-decision --mode ask --policy-file .coderoom/permission_policy.json";
const CODE_ROOM_HOOK_SENTINEL: &str = "__coderoom-hook-decision";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ClaudeHookInstall {
    pub(super) settings_path: PathBuf,
    pub(super) marker_path: PathBuf,
    pub(super) settings_status: HookWriteStatus,
    pub(super) marker_status: HookWriteStatus,
    pub(super) backup_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HookWriteStatus {
    Created,
    Updated,
    Unchanged,
}

pub(super) fn install_or_upgrade(project_root: &Path) -> Result<ClaudeHookInstall> {
    let claude_dir = project_root.join(CLAUDE_DIR);
    std::fs::create_dir_all(&claude_dir)
        .with_context(|| format!("creating {}", claude_dir.display()))?;
    let settings_path = claude_dir.join(SETTINGS_FILE);
    let marker_path = claude_dir.join(MARKER_FILE);
    let existing = match std::fs::read_to_string(&settings_path) {
        Ok(content) => Some(content),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(error).with_context(|| format!("reading {}", settings_path.display()));
        }
    };
    let mut settings = match existing.as_deref() {
        Some(content) => serde_json::from_str(content)
            .with_context(|| format!("parsing {}", settings_path.display()))?,
        None => json!({}),
    };
    merge_coderoom_hook(&mut settings)?;
    let rendered = format!("{}\n", serde_json::to_string_pretty(&settings)?);
    let (settings_status, backup_path) =
        write_settings_if_changed(&settings_path, existing.as_deref(), &rendered)?;
    let marker_status = write_marker_if_changed(&marker_path)?;

    Ok(ClaudeHookInstall {
        settings_path,
        marker_path,
        settings_status,
        marker_status,
        backup_path,
    })
}

fn write_settings_if_changed(
    path: &Path,
    existing: Option<&str>,
    rendered: &str,
) -> Result<(HookWriteStatus, Option<PathBuf>)> {
    if existing == Some(rendered) {
        return Ok((HookWriteStatus::Unchanged, None));
    }
    let backup_path = if path.exists() {
        let backup = backup_path(path);
        std::fs::copy(path, &backup)
            .with_context(|| format!("backing up {} to {}", path.display(), backup.display()))?;
        Some(backup)
    } else {
        None
    };
    std::fs::write(path, rendered).with_context(|| format!("writing {}", path.display()))?;
    let status = if existing.is_some() {
        HookWriteStatus::Updated
    } else {
        HookWriteStatus::Created
    };
    Ok((status, backup_path))
}

fn write_marker_if_changed(path: &Path) -> Result<HookWriteStatus> {
    let marker = format!("{}\n", serde_json::to_string_pretty(&marker_value())?);
    match std::fs::read_to_string(path) {
        Ok(existing) if existing == marker => Ok(HookWriteStatus::Unchanged),
        Ok(_) => {
            std::fs::write(path, marker).with_context(|| format!("writing {}", path.display()))?;
            Ok(HookWriteStatus::Updated)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::write(path, marker).with_context(|| format!("writing {}", path.display()))?;
            Ok(HookWriteStatus::Created)
        }
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

fn backup_path(path: &Path) -> PathBuf {
    let stamp = chrono::Local::now().format("%Y%m%d%H%M%S");
    path.with_file_name(format!("{SETTINGS_FILE}.bak.{stamp}"))
}

fn marker_value() -> Value {
    json!({
        "generated_by": "codeRoom",
        "safe_to_edit": true,
        "docs": "docs/getting-started.md#claude-code-hooks",
        "managed_files": [".claude/settings.json"]
    })
}

fn merge_coderoom_hook(settings: &mut Value) -> Result<()> {
    let Some(settings_object) = settings.as_object_mut() else {
        bail!("cannot merge .claude/settings.json: root JSON value must be an object");
    };
    let hooks = settings_object.entry("hooks").or_insert_with(|| json!({}));
    let Some(hooks_object) = hooks.as_object_mut() else {
        bail!("cannot merge .claude/settings.json: `hooks` must be an object");
    };
    let pre_tool_use = hooks_object
        .entry("PreToolUse")
        .or_insert_with(|| json!([]));
    let Some(entries) = pre_tool_use.as_array_mut() else {
        bail!("cannot merge .claude/settings.json: `hooks.PreToolUse` must be an array");
    };

    remove_existing_coderoom_hooks(entries)?;
    entries.push(desired_pretooluse_entry());
    Ok(())
}

fn remove_existing_coderoom_hooks(entries: &mut Vec<Value>) -> Result<()> {
    let mut retained = Vec::with_capacity(entries.len());
    for mut entry in std::mem::take(entries) {
        let Some(object) = entry.as_object_mut() else {
            retained.push(entry);
            continue;
        };
        let Some(hooks) = object.get_mut("hooks") else {
            retained.push(entry);
            continue;
        };
        let Some(hook_array) = hooks.as_array_mut() else {
            bail!("cannot merge .claude/settings.json: PreToolUse entry `hooks` must be an array");
        };
        hook_array.retain(|hook| !is_coderoom_hook(hook));
        if !hook_array.is_empty() {
            retained.push(entry);
        }
    }
    *entries = retained;
    Ok(())
}

fn is_coderoom_hook(hook: &Value) -> bool {
    hook.get("command")
        .and_then(Value::as_str)
        .is_some_and(|command| command.contains(CODE_ROOM_HOOK_SENTINEL))
}

fn desired_pretooluse_entry() -> Value {
    json!({
        "matcher": "*",
        "hooks": [
            {
                "type": "command",
                "command": HOOK_COMMAND
            }
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_preserves_existing_hooks_and_replaces_old_coderoom_hook() {
        let mut settings = json!({
            "hooks": {
                "PreToolUse": [
                    {
                        "matcher": "Bash",
                        "hooks": [
                            {"type": "command", "command": "echo keep"},
                            {"type": "command", "command": "old-cr __coderoom-hook-decision"}
                        ]
                    }
                ]
            }
        });

        merge_coderoom_hook(&mut settings).unwrap();

        let entries = settings["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries[0]["hooks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|hook| hook["command"] == "echo keep"));
        assert_eq!(
            entries[1]["hooks"][0]["command"],
            "cr __coderoom-hook-decision --mode ask --policy-file .coderoom/permission_policy.json"
        );
    }
}
