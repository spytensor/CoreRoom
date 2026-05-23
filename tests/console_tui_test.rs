//! Ratatui console shell fixtures.

use coreroom::console_snapshot::{ConversationTurn, ConversationVisibility, CoreRoomSnapshot};
use coreroom::console_tui::render_snapshot_to_text;

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
    assert!(rendered.contains("user <-> @host"));
    assert!(rendered.contains("WO-0242"));
    assert!(rendered.contains("Define CoreRoomSnapshot schema"));
}

#[test]
fn console_shell_keeps_internal_delegation_out_of_public_transcript() {
    let snapshot = snapshot();
    let rendered = render_snapshot_to_text(&snapshot, 180, 48).expect("rendered console");

    assert!(rendered.contains("hidden delegation: 3 internal / 1 side-rail"));
    assert!(rendered.contains("user <-> @host"));
    assert!(!rendered.contains("Side rail: active tracker #238"));
    assert!(!rendered.contains("xray:thread-v08-console-fixture/reviewer"));
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
