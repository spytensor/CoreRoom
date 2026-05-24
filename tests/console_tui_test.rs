//! Ratatui console shell fixtures.

use coreroom::console_actions::route_console_action;
use coreroom::console_navigation::{ConsoleNavigator, ConsoleView};
use coreroom::console_snapshot::{ConversationTurn, ConversationVisibility, CoreRoomSnapshot};
use coreroom::console_tui::{
    render_snapshot_to_text, render_snapshot_to_text_with_action_overlay,
    render_snapshot_to_text_with_avatar_pack, render_snapshot_to_text_with_nav,
    render_snapshot_to_text_with_nav_and_avatar_pack,
};
use coreroom::host_action::{ActionIntent, HostActionKind, HostActionRequest};
use coreroom::role_avatar::{role_label, RoleAvatarPack};

fn snapshot() -> CoreRoomSnapshot {
    toml::from_str(include_str!("fixtures/console_snapshot_v08.toml")).expect("snapshot")
}

#[test]
fn console_shell_renders_core_snapshot_facts() {
    let snapshot = snapshot();
    let rendered = render_snapshot_to_text(&snapshot, 180, 48).expect("rendered console");

    assert!(rendered.contains("CoreRoom"));
    assert!(rendered.contains("spytensor/CoreRoom"));
    assert!(rendered.contains("Tracker #238"));
    assert!(rendered.contains("Overview"));
    assert!(rendered.contains("Roles"));
    assert!(rendered.contains("WorkOrders"));
    assert!(rendered.contains("Gates"));
    assert!(rendered.contains("Evidence"));
    assert!(rendered.contains("Sources"));
    assert!(rendered.contains("Conversation"));
    assert!(rendered.contains("Control Rail"));
    assert!(rendered.contains("@user <-> @host"));
    assert!(rendered.contains("◉ @host"));
    assert!(rendered.contains("◎ @reviewer"));
    assert!(rendered.contains("WO-0242"));
    assert!(rendered.contains("Define CoreRoomSnapshot schema"));
}

#[test]
fn console_shell_keeps_internal_delegation_out_of_public_transcript() {
    let snapshot = snapshot();
    let rendered = render_snapshot_to_text(&snapshot, 180, 48).expect("rendered console");

    assert!(rendered.contains("Internal work:"));
    assert!(rendered.contains("3 hidden turns"));
    assert!(rendered.contains("@user <-> @host"));
    assert!(rendered.contains("Host-managed task cards"));
    assert!(rendered.contains("◎ @reviewer"));
    assert!(!rendered.contains("Side rail: active tracker #238"));
    assert!(rendered.contains("detail: xray:thread-v08-console-fixture/reviewer"));
}

#[test]
fn console_shell_supports_opt_in_nerd_font_role_avatars() {
    let snapshot = snapshot();
    let rendered =
        render_snapshot_to_text_with_avatar_pack(&snapshot, 180, 48, RoleAvatarPack::NerdFont)
            .expect("rendered console");

    assert!(rendered.contains(&role_label("host", "host", RoleAvatarPack::NerdFont)));
    assert!(rendered.contains(&role_label("reviewer", "host", RoleAvatarPack::NerdFont)));
    assert!(rendered.contains("@user <-> @host"));
}

#[test]
fn console_shell_can_show_user_addressed_specialist_turns() {
    let mut snapshot = snapshot();
    snapshot.conversation.public_turns.push(ConversationTurn {
        speaker: "security".to_owned(),
        body: "User-addressed security answer.".to_owned(),
        visibility: ConversationVisibility::PublicTranscript,
    });
    let rendered = render_snapshot_to_text(&snapshot, 180, 48).expect("rendered console");

    assert!(rendered.contains("@security"));
    assert!(rendered.contains("User-addressed security answer."));
}

#[test]
fn console_shell_renders_active_navigation_view_and_detail_source() {
    let snapshot = snapshot();
    let nav = ConsoleNavigator {
        active_view: ConsoleView::WorkOrders,
        detail_open: true,
        ..ConsoleNavigator::default()
    };
    let rendered = render_snapshot_to_text_with_nav_and_avatar_pack(
        &snapshot,
        180,
        48,
        &nav,
        RoleAvatarPack::Safe,
    )
    .expect("rendered console");

    assert!(rendered.contains("workorders detail"));
    assert!(rendered.contains("> WO-0242"));
    assert!(rendered.contains("tracker:#238"));
    assert!(!rendered.contains("Public conversation:"));
}

#[test]
fn console_roles_view_preserves_names_with_safe_avatars() {
    let snapshot = snapshot();
    let nav = ConsoleNavigator {
        active_view: ConsoleView::Roles,
        ..ConsoleNavigator::default()
    };
    let rendered =
        render_snapshot_to_text_with_nav(&snapshot, 180, 48, &nav).expect("rendered console");

    assert!(rendered.contains("◉ @host"));
    assert!(rendered.contains("◆ @security"));
    assert!(rendered.contains("@engineer"));
}

#[test]
fn console_shell_renders_permission_overlay_as_center_modal() {
    let snapshot = snapshot();
    let overlay = route_console_action(HostActionRequest::new(
        "HA-260",
        "host",
        "host",
        ActionIntent::Execute,
        HostActionKind::UpdateTracker,
        "WO-0260",
        "Console requests tracker update.",
    ))
    .expect("overlay");
    let rendered = render_snapshot_to_text_with_action_overlay(
        &snapshot,
        180,
        48,
        &ConsoleNavigator::default(),
        &overlay,
    )
    .expect("rendered console");

    assert!(rendered.contains("Host Action"));
    assert!(rendered.contains("CONFIRMATION REQUIRED"));
    assert!(rendered.contains("Action: update-tracker"));
    assert!(rendered.contains("Can execute: false"));
}
