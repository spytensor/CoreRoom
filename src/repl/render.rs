use crossterm::style::{Color, Stylize};
use crossterm::terminal;
use tracing::debug;

use crate::crep::CrepEvent;
use crate::output;

use super::text::{one_line, truncate_inline};

/// One-quarter block — thin, single-column vertical bar painted in a
/// role's stable color and prefixed onto every event line so the user
/// can tell at a glance which role is speaking, even on dense streams
/// where many `@`-tokens are visible at once.
pub(super) const GUTTER: &str = "▎";

/// Trace-style gutter: same color as the role's main gutter but dimmer
/// so tool-call lines visually nest under their role's spoke without
/// drowning it out.
fn trace_gutter(role_paint: Color) -> String {
    GUTTER.with(role_paint).to_string()
}

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
    render_event_line_at_width(
        event,
        host_role,
        terminal::size().map_or(80, |(cols, _)| cols.into()),
    )
}

pub(super) fn render_event_line_at_width(
    event: &CrepEvent,
    host_role: &str,
    width: usize,
) -> String {
    match event {
        CrepEvent::RoleStarted {
            role,
            engine,
            model,
            ..
        } => {
            let model = started_model_label(engine, model);
            let role_paint = output::role_color(role, host_role);
            format!(
                "{} {}",
                GUTTER.with(role_paint),
                format!("@{role} ready · model={model}")
                    .with(output::DIM)
                    .italic()
            )
        }
        CrepEvent::WorkTitle { role, title, .. } => {
            let role_paint = output::role_color(role, host_role);
            format!(
                "{} {}",
                GUTTER.with(role_paint),
                format!("@{role} work · {title}").with(output::DIM).italic()
            )
        }
        CrepEvent::RoleSpoke {
            role,
            text,
            cost_usd,
            ..
        } => {
            let _ = cost_usd;
            super::markdown::render_role_markdown(role, host_role, text, width)
        }
        CrepEvent::RoleOutputDelta {
            role, text_delta, ..
        } => super::markdown::render_role_markdown(role, host_role, text_delta, width),
        CrepEvent::ToolCallProposed {
            role,
            tool_name,
            tool_input,
            ..
        } => {
            let summary = summarize_tool_input(tool_input);
            let role_paint = output::role_color(role, host_role);
            format!(
                "{} {} @{role} · {}",
                trace_gutter(role_paint),
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
            let role_paint = output::role_color(role, host_role);
            let glyph = if *ok {
                "✓".with(output::OK)
            } else {
                "✗".with(output::BAD)
            };
            format!(
                "{} {glyph} @{role} · {}",
                trace_gutter(role_paint),
                truncate_inline(&one_line(output_summary), 100).with(output::DIM)
            )
        }
        CrepEvent::PermissionDenied {
            role,
            tool_name,
            reason,
            ..
        } => {
            // Permission events get the warn color on the gutter too —
            // they're the one CREP variant where the role color is less
            // important than the security signal.
            format!(
                "{} {} @{role} · {}",
                GUTTER.with(output::WARN),
                "⊘".with(output::WARN),
                format!("{tool_name} denied: {reason}").with(output::DIM),
            )
        }
        CrepEvent::RoleStopped { role, reason, .. } => {
            let role_paint = output::role_color(role, host_role);
            format!(
                "{} {}",
                GUTTER.with(role_paint),
                format!("@{role} stopped: {reason:?}")
                    .with(output::DIM)
                    .italic()
            )
        }
        // Turn lifecycle traces stay intentionally terse; WorkCards and
        // status lines carry the richer activity state.
        CrepEvent::TurnDispatched {
            role,
            queue_position,
            ..
        } => {
            let role_paint = output::role_color(role, host_role);
            let queued = if *queue_position == 0 {
                "starting".to_owned()
            } else {
                format!("queued · {queue_position} ahead")
            };
            format!(
                "{} {}",
                GUTTER.with(role_paint),
                format!("@{role} {queued}").with(output::DIM).italic()
            )
        }
        CrepEvent::TurnInterrupted { role, source, .. } => {
            let role_paint = output::role_color(role, host_role);
            format!(
                "{} {}",
                GUTTER.with(role_paint),
                format!("@{role} interrupted ({source:?})")
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
        return format!("`{}`", truncate_inline(&one_line(cmd), 80));
    }
    if let Some(path) = input.get("file_path").and_then(|v| v.as_str()) {
        return truncate_inline(&one_line(path), 80);
    }
    if let Some(obj) = input.as_object() {
        let keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        return format!("({})", keys.join(", "));
    }
    String::new()
}
