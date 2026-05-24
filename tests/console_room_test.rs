//! Unified live room bridge fixtures.

use coreroom::console_actions::route_console_action;
use coreroom::console_composer::ComposerState;
use coreroom::console_room::{
    live_room_command_specs, DispatchOrigin, LiveRoomAction, LiveRoomBridge,
};
use coreroom::console_snapshot::{ConversationVisibility, CoreRoomSnapshot, RoleLaneState};
use coreroom::console_tui::render_live_room_to_text;
use coreroom::host_action::{ActionIntent, HostActionKind, HostActionRequest};

fn snapshot() -> CoreRoomSnapshot {
    toml::from_str(include_str!("fixtures/console_snapshot_v08.toml")).expect("snapshot")
}

#[test]
fn live_room_routes_bare_text_to_host_and_preserves_public_user_turn() {
    let mut snapshot = snapshot();
    let mut bridge = LiveRoomBridge::from_snapshot(&snapshot);

    let action = bridge
        .submit(&mut snapshot, "review the unified room bridge")
        .expect("submit");

    assert_eq!(
        action,
        LiveRoomAction::Dispatch {
            target_role: "host".to_owned(),
            text: "review the unified room bridge".to_owned(),
            origin: DispatchOrigin::BareUserText,
        }
    );
    assert_eq!(snapshot.runtime.active_role.as_deref(), Some("host"));
    assert!(snapshot.conversation.public_turns.iter().any(|turn| {
        turn.speaker == "user"
            && turn.body == "review the unified room bridge"
            && turn.visibility == ConversationVisibility::PublicTranscript
    }));
}

#[test]
fn live_room_routes_explicit_role_mentions_with_existing_repl_semantics() {
    let mut snapshot = snapshot();
    let mut bridge = LiveRoomBridge::from_snapshot(&snapshot);

    let action = bridge
        .submit(&mut snapshot, "@reviewer check the visibility contract")
        .expect("submit");

    assert_eq!(
        action,
        LiveRoomAction::Dispatch {
            target_role: "reviewer".to_owned(),
            text: "check the visibility contract".to_owned(),
            origin: DispatchOrigin::ExplicitRoleMention,
        }
    );
    assert!(snapshot.runtime.roles.iter().any(|role| {
        role.role == "reviewer"
            && role.state == RoleLaneState::Working
            && role.last_activity.as_deref() == Some("queued by live room composer")
    }));
    assert!(snapshot
        .conversation
        .public_turns
        .iter()
        .any(|turn| turn.body == "@reviewer check the visibility contract"));
}

#[test]
fn live_room_blocks_runtime_only_slash_commands_with_clear_message() {
    let mut snapshot = snapshot();
    let mut bridge = LiveRoomBridge::from_snapshot(&snapshot);

    let action = bridge
        .submit(&mut snapshot, "/journal reviewer")
        .expect("submit");

    match action {
        LiveRoomAction::UnsupportedSlash { command, message } => {
            assert_eq!(command, "journal");
            assert!(message.contains("unified room"));
            assert!(message.contains("cr start"));
        }
        other => panic!("unexpected action: {other:?}"),
    }
    assert!(snapshot
        .conversation
        .public_turns
        .iter()
        .any(|turn| turn.speaker == "host" && turn.body.contains("not yet available")));
}

#[test]
fn permission_overlay_stays_out_of_snapshot_conversation() {
    let snapshot = snapshot();
    let before = snapshot.conversation.clone();
    let mut bridge = LiveRoomBridge::from_snapshot(&snapshot);
    let overlay = route_console_action(HostActionRequest::new(
        "HA-live-room",
        "host",
        "host",
        ActionIntent::Execute,
        HostActionKind::UpdateTracker,
        "WO-0305",
        "Live room bridge asks for tracker update.",
    ))
    .expect("overlay");

    bridge.set_permission_overlay(overlay);

    assert!(bridge.permission_overlay().is_some());
    assert_eq!(snapshot.conversation, before);
}

#[test]
fn live_room_frame_renders_dashboard_conversation_and_composer_together() {
    let mut snapshot = snapshot();
    let mut bridge = LiveRoomBridge::from_snapshot(&snapshot);
    let mut composer = ComposerState::new(
        bridge.roles().to_vec(),
        live_room_command_specs(),
        "type a task - @role - /help - /exit",
    );
    composer.paste_str("@reviewer check the bridge");
    let _ = bridge
        .submit(&mut snapshot, "@reviewer check the bridge")
        .expect("submit");

    let rendered =
        render_live_room_to_text(&snapshot, 180, 52, &composer, &bridge).expect("render");

    assert!(rendered.contains("Conversation"));
    assert!(rendered.contains("Host-managed task cards"));
    assert!(rendered.contains("Composer"));
    assert!(rendered.contains("bridge queued for @reviewer"));
    assert!(rendered.contains("@reviewer check the bridge"));
}
