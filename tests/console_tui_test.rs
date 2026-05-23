//! Ratatui console shell fixtures.

use coreroom::console_snapshot::CoreRoomSnapshot;
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

    assert!(rendered.contains("internal delegations hidden: 3"));
    assert!(rendered.contains("user <-> @host"));
    assert!(!rendered.contains("xray:thread-v08-console-fixture/reviewer"));
}
