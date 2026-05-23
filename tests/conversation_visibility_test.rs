//! Public transcript and internal delegation visibility fixtures.

use coreroom::console_snapshot::ConversationVisibility;
use coreroom::conversation_visibility::{
    decide_visibility, ConversationVisibilityInput, HostSurfaceReason,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VisibilityFixture {
    cases: Vec<VisibilityCase>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VisibilityCase {
    name: String,
    input: ConversationVisibilityInput,
    expected_visibility: ConversationVisibility,
    expected_public: bool,
    expected_side_rail: bool,
    expected_xray: bool,
}

#[test]
fn visibility_fixture_proves_public_and_internal_routing() {
    let fixture: VisibilityFixture =
        toml::from_str(include_str!("fixtures/conversation_visibility.toml")).expect("fixture");

    for case in fixture.cases {
        let decision = decide_visibility(&case.input).expect("decision");
        assert_eq!(
            decision.visibility, case.expected_visibility,
            "{}",
            case.name
        );
        assert_eq!(
            decision.public_transcript, case.expected_public,
            "{}",
            case.name
        );
        assert_eq!(
            decision.side_rail_activity, case.expected_side_rail,
            "{}",
            case.name
        );
        assert_eq!(decision.xray_available, case.expected_xray, "{}", case.name);
        assert!(!decision.reason.is_empty(), "{}", case.name);
    }
}

#[test]
fn host_internal_delegation_does_not_pollute_public_transcript() {
    let decision = decide_visibility(&ConversationVisibilityInput::HostToRole {
        role: "security".to_owned(),
        work_order: Some("WO-0244".to_owned()),
    })
    .expect("decision");

    assert_eq!(
        decision.visibility,
        ConversationVisibility::InternalDelegation
    );
    assert!(!decision.public_transcript);
    assert!(decision.side_rail_activity);
    assert!(decision.xray_available);
}

#[test]
fn host_can_surface_critical_role_output_publicly() {
    let decision = decide_visibility(&ConversationVisibilityInput::RoleToHost {
        role: "security".to_owned(),
        surfaced_by_host: Some(HostSurfaceReason::Veto),
    })
    .expect("decision");

    assert_eq!(
        decision.visibility,
        ConversationVisibility::PublicTranscript
    );
    assert!(decision.public_transcript);
    assert!(!decision.side_rail_activity);
    assert!(decision.reason.contains("veto"));
}

#[test]
fn console_snapshot_public_turns_reject_internal_delegation_pollution() {
    let mut snapshot: coreroom::console_snapshot::CoreRoomSnapshot =
        toml::from_str(include_str!("fixtures/console_snapshot_v08.toml")).expect("snapshot");
    snapshot
        .conversation
        .public_turns
        .push(coreroom::console_snapshot::ConversationTurn {
            speaker: "reviewer".to_owned(),
            body: "Internal review detail should not appear in public turns.".to_owned(),
            visibility: ConversationVisibility::InternalDelegation,
        });

    let err = snapshot
        .validate()
        .expect_err("internal public turn rejected");
    assert!(err
        .to_string()
        .contains("conversation.publicTurns cannot contain"));
}
