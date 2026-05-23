//! Public conversation panel projection fixtures.

use coreroom::console_conversation::build_public_conversation;
use coreroom::console_snapshot::{ConversationTurn, ConversationVisibility, CoreRoomSnapshot};

fn snapshot() -> CoreRoomSnapshot {
    toml::from_str(include_str!("fixtures/console_snapshot_v08.toml")).expect("snapshot")
}

#[test]
fn public_conversation_excludes_side_rail_and_internal_activity() {
    let snapshot = snapshot();
    let panel = build_public_conversation(&snapshot);

    assert_eq!(panel.hidden_internal_count, 3);
    assert_eq!(panel.side_rail_turn_count, 1);
    assert!(panel
        .turns
        .iter()
        .all(|turn| turn.visibility == ConversationVisibility::PublicTranscript));
    assert!(!panel
        .turns
        .iter()
        .any(|turn| turn.body.starts_with("Side rail:")));
    assert_eq!(panel.internal_activity.len(), 2);
}

#[test]
fn user_addressed_specialist_turn_can_be_public() {
    let mut snapshot = snapshot();
    snapshot.conversation.public_turns.push(ConversationTurn {
        speaker: "security".to_owned(),
        body: "User-addressed security answer.".to_owned(),
        visibility: ConversationVisibility::PublicTranscript,
    });

    let panel = build_public_conversation(&snapshot);
    assert!(panel.has_public_specialist_turn(&snapshot.runtime.host_role));
    assert!(panel
        .turns
        .iter()
        .any(|turn| turn.speaker == "security" && turn.body == "User-addressed security answer."));
}
