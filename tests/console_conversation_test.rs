//! Live room conversation projection fixtures.

use coreroom::console_conversation::{
    build_live_room_conversation, build_public_conversation, ConversationAuthority,
    LiveRoomTurnKind,
};
use coreroom::console_snapshot::{
    ConversationTurn, ConversationVisibility, CoreRoomSnapshot, InternalDelegationState,
};

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

#[test]
fn live_room_model_classifies_public_turns_and_internal_task_cards() {
    let mut snapshot = snapshot();
    snapshot.conversation.public_turns.push(ConversationTurn {
        speaker: "security".to_owned(),
        body: "User-addressed security answer.".to_owned(),
        visibility: ConversationVisibility::PublicTranscript,
    });

    let panel = build_live_room_conversation(&snapshot);

    assert_eq!(panel.hidden_internal_count, 3);
    assert_eq!(panel.side_rail_turn_count, 1);
    assert!(panel
        .public_turns
        .iter()
        .any(|turn| turn.speaker == "user" && turn.kind == LiveRoomTurnKind::User));
    assert!(panel
        .public_turns
        .iter()
        .any(|turn| turn.speaker == "host" && turn.kind == LiveRoomTurnKind::Host));
    assert!(panel.public_turns.iter().any(|turn| {
        turn.speaker == "security" && turn.kind == LiveRoomTurnKind::DirectSpecialist
    }));
    assert!(panel.has_public_specialist_turn());
    assert_eq!(panel.task_cards.len(), 2);
    assert!(panel.task_cards.iter().any(|card| {
        card.role == "reviewer"
            && card.work_order.as_deref() == Some("WO-0242")
            && card.state == InternalDelegationState::Reviewing
            && card.xray_ref.as_deref() == Some("xray:thread-v08-console-fixture/reviewer")
    }));
}

#[test]
fn live_room_model_is_display_only_not_completion_evidence() {
    let panel = build_live_room_conversation(&snapshot());

    assert_eq!(panel.authority, ConversationAuthority::DisplayOnly);
    assert!(
        panel
            .task_cards
            .iter()
            .all(|card| !card.summary.contains("complete enough to merge")),
        "task cards should summarize activity, not become completion proof"
    );
}
