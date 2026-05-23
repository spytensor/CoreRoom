//! Console snapshot contract fixtures.

use coreroom::console_snapshot::{
    CoreRoomSnapshot, HealthSeverity, SessionFreshness, StatusState, WorkLifecycle,
    CONSOLE_SNAPSHOT_SCHEMA_VERSION,
};

#[test]
fn console_snapshot_fixture_roundtrips_and_validates() {
    let fixture = include_str!("fixtures/console_snapshot_v08.toml");
    let snapshot: CoreRoomSnapshot = toml::from_str(fixture).expect("parse fixture");

    snapshot.validate().expect("valid console snapshot");
    assert_eq!(snapshot.schema_version, CONSOLE_SNAPSHOT_SCHEMA_VERSION);
    assert_eq!(snapshot.project.project, "CoreRoom");
    assert_eq!(snapshot.project.tracker_issue, 238);
    assert_eq!(snapshot.github.tracker_issue, 238);
    assert_eq!(snapshot.runtime.session_state, SessionFreshness::Resumed);
    assert!(snapshot.runtime.roles.len() >= 4);
    assert!(snapshot.conversation.internal_delegation_count > 0);
    assert!(snapshot
        .conversation
        .public_turns
        .iter()
        .all(|turn| matches!(
            turn.visibility,
            coreroom::console_snapshot::ConversationVisibility::PublicTranscript
                | coreroom::console_snapshot::ConversationVisibility::SideRail
        )));

    let encoded = toml::to_string_pretty(&snapshot).expect("encode fixture");
    let decoded: CoreRoomSnapshot = toml::from_str(&encoded).expect("decode roundtrip");
    assert_eq!(decoded, snapshot);
}

#[test]
fn console_snapshot_fixture_preserves_actionable_states() {
    let snapshot: CoreRoomSnapshot =
        toml::from_str(include_str!("fixtures/console_snapshot_v08.toml")).expect("fixture");

    assert!(snapshot.work.iter().any(|work| {
        work.lifecycle == WorkLifecycle::MergedTrackerStale
            && work.tracker_state == StatusState::Blocking
    }));
    assert!(snapshot
        .work
        .iter()
        .any(|work| work.lifecycle == WorkLifecycle::Blocked));
    assert!(snapshot
        .alerts
        .iter()
        .any(|alert| alert.severity == HealthSeverity::Blocking));
    assert!(snapshot
        .sources
        .iter()
        .any(|source| source.status == coreroom::console_snapshot::SourceHealthState::Stale));
}

#[test]
fn console_snapshot_rejects_unsupported_schema_version() {
    let mut snapshot: CoreRoomSnapshot =
        toml::from_str(include_str!("fixtures/console_snapshot_v08.toml")).expect("fixture");
    snapshot.schema_version = CONSOLE_SNAPSHOT_SCHEMA_VERSION + 1;

    let err = snapshot.validate().expect_err("unsupported schema");
    assert!(err
        .to_string()
        .contains("unsupported CoreRoomSnapshot schemaVersion"));
}
