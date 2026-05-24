//! Console action overlay fixtures.

use coreroom::console_actions::{
    action_alert, build_permission_overlay, route_console_action, PermissionOverlayPlacement,
};
use coreroom::console_snapshot::StatusState;
use coreroom::host_action::{
    evaluate_host_action, ActionConfirmationStatus, ActionIntent, ActionOutcome, HostActionKind,
    HostActionRequest,
};

#[test]
fn confirmation_required_action_renders_explicit_center_overlay() {
    let overlay = route_console_action(host_request(
        "HA-260-confirm",
        "host",
        ActionIntent::Execute,
        HostActionKind::UpdateTracker,
    ))
    .expect("overlay");

    assert_eq!(overlay.placement, PermissionOverlayPlacement::CenterModal);
    assert_eq!(overlay.title, "CONFIRMATION REQUIRED");
    assert_eq!(overlay.outcome, ActionOutcome::ConfirmationRequired);
    assert_eq!(
        overlay.confirmation_status,
        ActionConfirmationStatus::RequiredPending
    );
    assert!(overlay.confirmation_required);
    assert!(!overlay.can_execute);
    assert!(overlay.audit_recorded);
    assert_eq!(overlay.status, StatusState::Warn);
    assert!(overlay
        .lines
        .iter()
        .any(|line| line.contains("explicit user confirmation")));
}

#[test]
fn confirmed_mutating_action_is_allowed_and_audited() {
    let mut request = host_request(
        "HA-260-allowed",
        "host",
        ActionIntent::Execute,
        HostActionKind::UpdateTracker,
    );
    request.confirmed_by = Some("user".to_owned());
    let result = evaluate_host_action(request).expect("action");
    let overlay = build_permission_overlay(&result);

    assert_eq!(overlay.title, "ACTION ALLOWED");
    assert_eq!(overlay.status, StatusState::Ok);
    assert!(overlay.can_execute);
    assert!(!overlay.confirmation_required);
    assert!(overlay.audit_recorded);
    assert!(action_alert(&overlay).is_none());
}

#[test]
fn non_host_execution_is_denied_and_stays_visible_as_alert() {
    let overlay = route_console_action(host_request(
        "HA-260-denied",
        "engineer",
        ActionIntent::Execute,
        HostActionKind::UpdateTracker,
    ))
    .expect("overlay");
    let alert = action_alert(&overlay).expect("alert");

    assert_eq!(overlay.title, "ACTION DENIED");
    assert_eq!(overlay.status, StatusState::Blocking);
    assert!(!overlay.can_execute);
    assert_eq!(alert.source, "host-action:HA-260-denied");
    assert!(alert
        .next_action
        .as_deref()
        .is_some_and(|action| action.contains("non-host")));
}

#[test]
fn blocked_action_remains_visible_with_rollback_and_audit() {
    let mut request = host_request(
        "HA-260-blocked",
        "host",
        ActionIntent::Execute,
        HostActionKind::RegisterSource,
    );
    request.attempt.blocker = Some("source pin drift needs user decision".to_owned());

    let overlay = route_console_action(request).expect("overlay");
    let alert = action_alert(&overlay).expect("alert");

    assert_eq!(overlay.title, "ACTION BLOCKED");
    assert_eq!(overlay.status, StatusState::Blocking);
    assert!(overlay.audit_recorded);
    assert!(overlay.rollback_hint.contains("Source Registry"));
    assert!(alert.title.contains("BLOCKED"));
}

fn host_request(
    id: &str,
    actor: &str,
    intent: ActionIntent,
    kind: HostActionKind,
) -> HostActionRequest {
    HostActionRequest::new(
        id,
        actor,
        "host",
        intent,
        kind,
        "WO-0260",
        "Exercise console action overlay.",
    )
}
