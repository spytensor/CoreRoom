//! CREP-to-console reducer fixtures.

use coreroom::console_snapshot::ConversationVisibility;
use coreroom::console_state::{reduce_jsonl_lines, ToolActivityState};

#[test]
fn crep_fixture_reduces_to_deterministic_conversation_viewport() {
    let report = reduce_jsonl_lines("host", include_str!("fixtures/console_reducer_crep.jsonl"))
        .expect("reduce fixture");

    assert_eq!(report.skipped_malformed, 1);
    assert!(report.parsed_events > 8);
    let public = &report.state.conversation.public_turns;
    assert!(public
        .iter()
        .any(|turn| turn.speaker == "host" && turn.body.contains("I will plan v0.8")));
    assert!(
        !public
            .iter()
            .any(|turn| turn.body.contains("Reviewer internal finding")),
        "internal reviewer reply leaked into public transcript: {public:?}"
    );
    assert!(public
        .iter()
        .any(|turn| { turn.speaker == "security" && turn.body.contains("Blocked review phase") }));
    assert!(public
        .iter()
        .all(|turn| turn.visibility == ConversationVisibility::PublicTranscript));
}

#[test]
fn reducer_folds_stream_deltas_without_printing() {
    let report = reduce_jsonl_lines("host", include_str!("fixtures/console_reducer_crep.jsonl"))
        .expect("reduce fixture");
    let summary = report
        .state
        .stream_summaries
        .get("turn-host-1")
        .expect("host stream summary");

    assert_eq!(summary.role, "host");
    assert_eq!(summary.chunks, 2);
    assert!(summary.preview.contains("I will plan"));
}

#[test]
fn reducer_tracks_tools_permissions_and_internal_activity() {
    let report = reduce_jsonl_lines("host", include_str!("fixtures/console_reducer_crep.jsonl"))
        .expect("reduce fixture");

    assert!(report
        .state
        .tool_activity
        .iter()
        .any(|tool| { tool.role == "reviewer" && tool.state == ToolActivityState::Proposed }));
    assert!(report
        .state
        .tool_activity
        .iter()
        .any(|tool| { tool.role == "reviewer" && tool.state == ToolActivityState::ExecutedOk }));
    assert!(report.state.tool_activity.iter().any(|tool| {
        tool.role == "reviewer" && tool.state == ToolActivityState::PermissionDenied
    }));
    assert!(report.state.conversation.internal_delegation_count > 0);
    assert!(report
        .state
        .conversation
        .internal_activity
        .iter()
        .any(|activity| activity.role == "reviewer"));
}
