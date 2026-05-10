use crossterm::style::Stylize;
use tracing::debug;

use crate::crep::CrepEvent;
use crate::output;

use super::text::truncate_inline;

fn is_placeholder_model(model: &str) -> bool {
    let normalized = model.trim().to_ascii_lowercase();
    normalized.is_empty() || normalized == "model"
}

pub(super) fn started_model_label(engine: &str, model: &str) -> String {
    if !is_placeholder_model(model) {
        return model.to_owned();
    }
    match engine {
        "cc" => "Claude default".to_owned(),
        "codex" => "Codex default".to_owned(),
        "gemini" => "Gemini default".to_owned(),
        other => format!("{other} default"),
    }
}

pub(super) fn render_event(event: &CrepEvent, host_role: &str) {
    println!("{}", render_event_line(event, host_role));
    if let CrepEvent::RoleSpoke { role, cost_usd, .. } = event {
        debug!(role, cost_usd, "RoleSpoke rendered");
    }
}

pub(super) fn render_event_line(event: &CrepEvent, host_role: &str) -> String {
    match event {
        CrepEvent::RoleStarted {
            role,
            engine,
            model,
            ..
        } => {
            let model = started_model_label(engine, model);
            format!(
                "{}",
                format!("[@{role} ready · model={model}]")
                    .with(output::DIM)
                    .italic()
            )
        }
        CrepEvent::RoleSpoke {
            role,
            text,
            cost_usd,
            ..
        } => {
            let _ = cost_usd;
            format!("{} {}", output::role_token(role, host_role), text)
        }
        CrepEvent::ToolCallProposed {
            role,
            tool_name,
            tool_input,
            ..
        } => {
            let summary = summarize_tool_input(tool_input);
            format!(
                "  {} @{role} · {}",
                "↳".with(output::FADE),
                format!("{tool_name} {summary}").with(output::DIM),
            )
        }
        CrepEvent::ToolCallExecuted {
            role,
            ok,
            output_summary,
            ..
        } => {
            // Tool executed lines borrow the trace shape but swap in
            // ✓/✗ in their semantic colors per docs/colors.md §4.
            let glyph = if *ok {
                "✓".with(output::OK)
            } else {
                "✗".with(output::BAD)
            };
            format!(
                "  {glyph} @{role} · {}",
                output_summary.as_str().with(output::DIM)
            )
        }
        CrepEvent::PermissionDenied {
            role,
            tool_name,
            reason,
            ..
        } => {
            // ⊘ is `warn` per the glyph table; the message tier stays dim.
            format!(
                "  {} @{role} · {}",
                "⊘".with(output::WARN),
                format!("{tool_name} denied: {reason}").with(output::DIM),
            )
        }
        CrepEvent::RoleStopped { role, reason } => {
            format!(
                "{}",
                format!("[@{role} stopped: {reason:?}]")
                    .with(output::DIM)
                    .italic()
            )
        }
    }
}

pub(super) fn summarize_tool_input(input: &serde_json::Value) -> String {
    // Best-effort one-liner: if there's a "command", show it; if there's
    // a "file_path", show it; otherwise dump the JSON keys.
    if let Some(cmd) = input.get("command").and_then(|v| v.as_str()) {
        return format!("`{}`", truncate_inline(cmd, 80));
    }
    if let Some(path) = input.get("file_path").and_then(|v| v.as_str()) {
        return path.to_owned();
    }
    if let Some(obj) = input.as_object() {
        let keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        return format!("({})", keys.join(", "));
    }
    String::new()
}
