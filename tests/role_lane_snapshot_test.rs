//! Role lane runtime snapshot fixtures.

use coreroom::console_snapshot::{CoreRoomSnapshot, RoleLaneState, SessionFreshness};
use coreroom::console_state::reduce_jsonl_lines;
use serde::Serialize;

#[derive(Serialize)]
struct RoleLaneWrapper<'a> {
    roles: &'a [coreroom::console_snapshot::RoleRuntimeSnapshot],
}

#[test]
fn console_snapshot_fixture_covers_role_lane_states() {
    let snapshot: CoreRoomSnapshot =
        toml::from_str(include_str!("fixtures/console_snapshot_v08.toml")).expect("snapshot");
    snapshot.validate().expect("valid snapshot");

    let states = snapshot
        .runtime
        .roles
        .iter()
        .map(|role| role.state)
        .collect::<Vec<_>>();
    for expected in [
        RoleLaneState::Idle,
        RoleLaneState::Working,
        RoleLaneState::Reviewing,
        RoleLaneState::Blocked,
        RoleLaneState::WaitingApproval,
        RoleLaneState::WaitingUser,
        RoleLaneState::StaleSession,
    ] {
        assert!(states.contains(&expected), "missing {expected:?}");
    }

    let waiting = snapshot
        .runtime
        .roles
        .iter()
        .find(|role| role.role == "qa")
        .expect("qa role");
    assert!(waiting.waiting_approval);
    assert_eq!(waiting.permission_mode.as_deref(), Some("ask"));
    assert_eq!(waiting.session_state, SessionFreshness::Fresh);
}

#[test]
fn role_lane_snapshot_does_not_expose_session_ids_or_credential_material() {
    let snapshot: CoreRoomSnapshot =
        toml::from_str(include_str!("fixtures/console_snapshot_v08.toml")).expect("snapshot");
    let encoded = toml::to_string_pretty(&RoleLaneWrapper {
        roles: &snapshot.runtime.roles,
    })
    .expect("encode roles");
    let lower = encoded.to_ascii_lowercase();

    assert!(!encoded.contains("session_id"));
    assert!(!encoded.contains("sessionId"));
    assert!(encoded.contains("authority = [\"secrets\"]"));
    assert!(!lower.contains("secret_key"));
    assert!(!lower.contains("api_key"));
    assert!(!lower.contains("apikey"));
    assert!(!lower.contains("token"));
}

#[test]
fn console_state_projects_runtime_role_lanes() {
    let report = reduce_jsonl_lines("host", include_str!("fixtures/console_reducer_crep.jsonl"))
        .expect("reducer");
    let runtime = report
        .state
        .runtime_snapshot(Some("ask".to_owned()), SessionFreshness::Fresh);

    assert_eq!(runtime.host_role, "host");
    assert!(runtime
        .roles
        .iter()
        .any(|role| role.role == "host" && role.state == RoleLaneState::Idle));
    assert!(runtime
        .roles
        .iter()
        .any(|role| role.role == "reviewer" && role.state == RoleLaneState::Idle));
    assert_eq!(runtime.permission_mode.as_deref(), Some("ask"));
}
