//! Console action overlays backed by the host action layer.
//!
//! The full-screen console may surface actions, but it must not execute them
//! directly. Every request is evaluated through `host_action`, and the console
//! receives only an explicit overlay plus alert-compatible status.

use anyhow::Result;

use crate::console_snapshot::StatusState;
use crate::host_action::{
    evaluate_host_action, ActionConfirmationStatus, ActionOutcome, HostActionRequest,
    HostActionResult,
};

/// Where a permission/action prompt must render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionOverlayPlacement {
    /// Explicit centered modal. This must not be folded into a side rail.
    CenterModal,
}

/// Explicit console overlay for one host action decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsolePermissionOverlay {
    /// Overlay placement.
    pub placement: PermissionOverlayPlacement,
    /// Stable action id.
    pub action_id: String,
    /// Overlay title.
    pub title: String,
    /// Target object.
    pub target: String,
    /// Final outcome label.
    pub outcome: ActionOutcome,
    /// Confirmation status.
    pub confirmation_status: ActionConfirmationStatus,
    /// Whether execution may proceed.
    pub can_execute: bool,
    /// Whether explicit confirmation is still required.
    pub confirmation_required: bool,
    /// Whether an audit event exists for this decision.
    pub audit_recorded: bool,
    /// Compact health status for alerts.
    pub status: StatusState,
    /// User-facing message.
    pub message: String,
    /// Rollback hint copied from audit event.
    pub rollback_hint: String,
    /// Modal body lines.
    pub lines: Vec<String>,
}

/// Alert-compatible action row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleActionAlert {
    /// Alert id.
    pub id: String,
    /// Alert title.
    pub title: String,
    /// Alert status.
    pub status: StatusState,
    /// Source label.
    pub source: String,
    /// Recommended next action.
    pub next_action: Option<String>,
}

/// Route a console action through the host action layer and return an overlay.
pub fn route_console_action(request: HostActionRequest) -> Result<ConsolePermissionOverlay> {
    let result = evaluate_host_action(request)?;
    Ok(build_permission_overlay(&result))
}

/// Build an explicit overlay from a host action result.
#[must_use]
pub fn build_permission_overlay(result: &HostActionResult) -> ConsolePermissionOverlay {
    let status = status_for_outcome(result.decision.outcome);
    let title = overlay_title(result.decision.outcome).to_owned();
    let confirmation_required =
        result.decision.confirmation_status == ActionConfirmationStatus::RequiredPending;
    let audit_recorded = result.audit_event.action_id == result.request.id
        && !result.audit_event.output.trim().is_empty();
    let message = result.decision.reason.clone();
    let rollback_hint = result.audit_event.rollback_hint.clone();
    let lines = vec![
        format!("Action: {}", result.request.kind.label()),
        format!("Actor: @{}", result.request.actor_role),
        format!("Target: {}", result.request.target),
        format!("Outcome: {}", result.decision.outcome.label()),
        format!(
            "Confirmation: {}",
            result.decision.confirmation_status.label()
        ),
        format!("Can execute: {}", result.decision.can_execute),
        format!(
            "Audit: {}",
            if audit_recorded {
                "recorded"
            } else {
                "missing"
            }
        ),
        format!("Reason: {}", result.decision.reason),
        format!("Rollback: {rollback_hint}"),
    ];

    ConsolePermissionOverlay {
        placement: PermissionOverlayPlacement::CenterModal,
        action_id: result.request.id.clone(),
        title,
        target: result.request.target.clone(),
        outcome: result.decision.outcome,
        confirmation_status: result.decision.confirmation_status,
        can_execute: result.decision.can_execute,
        confirmation_required,
        audit_recorded,
        status,
        message,
        rollback_hint,
        lines,
    }
}

/// Convert an overlay into an alert row for denied, blocked, failed, or pending actions.
#[must_use]
pub fn action_alert(overlay: &ConsolePermissionOverlay) -> Option<ConsoleActionAlert> {
    (overlay.status != StatusState::Ok).then(|| ConsoleActionAlert {
        id: format!("action:{}", overlay.action_id),
        title: overlay.title.clone(),
        status: overlay.status,
        source: format!("host-action:{}", overlay.action_id),
        next_action: Some(overlay.message.clone()),
    })
}

fn overlay_title(outcome: ActionOutcome) -> &'static str {
    match outcome {
        ActionOutcome::Allowed => "ACTION ALLOWED",
        ActionOutcome::Proposed => "HOST REVIEW REQUIRED",
        ActionOutcome::ConfirmationRequired => "CONFIRMATION REQUIRED",
        ActionOutcome::HumanOnly => "HUMAN ONLY",
        ActionOutcome::Blocked => "ACTION BLOCKED",
        ActionOutcome::Forbidden => "ACTION DENIED",
        ActionOutcome::Failed => "ACTION FAILED",
    }
}

fn status_for_outcome(outcome: ActionOutcome) -> StatusState {
    match outcome {
        ActionOutcome::Allowed => StatusState::Ok,
        ActionOutcome::Proposed | ActionOutcome::ConfirmationRequired => StatusState::Warn,
        ActionOutcome::HumanOnly
        | ActionOutcome::Blocked
        | ActionOutcome::Forbidden
        | ActionOutcome::Failed => StatusState::Blocking,
    }
}
