//! v0.8 console data-plane dogfood validation.

use coreroom::console_health::overview_health_signals;
use coreroom::console_layout::{compute_console_layout, RightRailSectionKind};
use coreroom::console_snapshot::{ConversationVisibility, CoreRoomSnapshot, WorkLifecycle};
use coreroom::evidence_packet::{EvidencePacket, EvidenceStatus};
use coreroom::work_order::{WorkOrder, WorkOrderRoleAccess, WorkOrderStatus};

#[test]
fn v08_console_dogfood_flow_is_structurally_complete() {
    let work_order: WorkOrder =
        toml::from_str(include_str!("fixtures/v08_console_dogfood_work_order.toml"))
            .expect("work order fixture");
    let snapshot: CoreRoomSnapshot =
        toml::from_str(include_str!("fixtures/console_snapshot_v08.toml")).expect("snapshot");
    let evidence: EvidencePacket = toml::from_str(include_str!(
        "fixtures/v08_console_dogfood_evidence_packet.toml"
    ))
    .expect("evidence packet fixture");

    assert_eq!(work_order.id, "WO-0251");
    assert_eq!(work_order.github_issue, Some(251));
    assert_eq!(work_order.tracker_issue, Some(238));
    assert_eq!(work_order.status, WorkOrderStatus::Closed);
    assert_eq!(work_order.acceptance_criteria.len(), 8);
    assert_eq!(work_order.pull_request, Some(280));
    assert!(work_order
        .role_grants
        .iter()
        .any(|grant| grant.role == "backend" && grant.access == WorkOrderRoleAccess::Write));
    assert!(work_order
        .role_grants
        .iter()
        .any(|grant| grant.role == "reviewer" && grant.access == WorkOrderRoleAccess::ReadReview));

    snapshot.validate().expect("snapshot validates");
    assert_eq!(snapshot.project.tracker_issue, 238);
    assert_eq!(snapshot.runtime.host_role, "host");
    assert!(snapshot
        .work
        .iter()
        .any(|work| work.id == "WO-0251" && work.lifecycle == WorkLifecycle::Blocked));
    assert!(snapshot
        .work
        .iter()
        .any(|work| work.id == "WO-0206" && work.lifecycle == WorkLifecycle::MergedTrackerStale));

    let evidence_report = evidence.completion_report().expect("evidence report");
    assert_eq!(evidence_report.status, EvidenceStatus::Complete);
    assert!(evidence_report.missing.is_empty());
    assert_eq!(evidence.github_issue, 251);
    assert_eq!(evidence.pull_request, Some(280));
    assert_eq!(evidence.tracker_update.tracker_issue, Some(238));
    assert!(evidence.tracker_update.checkbox_updated);
    assert!(evidence.tracker_update.evidence_ledger_updated);
    assert!(evidence
        .tracker_update
        .milestone_ac_updated
        .contains(&"M-AC-10".to_owned()));

    assert_public_transcript_is_host_led(&snapshot);
    assert_layout_and_image_are_available(&snapshot);
    assert_health_signals_keep_negative_cases_visible(&snapshot);
    assert_transcript_covers_end_to_end_flow();
    assert_readiness_doc_names_v09_risks();
}

fn assert_public_transcript_is_host_led(snapshot: &CoreRoomSnapshot) {
    assert!(!snapshot.conversation.public_turns.is_empty());
    for turn in &snapshot.conversation.public_turns {
        assert!(
            matches!(
                turn.visibility,
                ConversationVisibility::PublicTranscript | ConversationVisibility::SideRail
            ),
            "public turns cannot contain internal/debug visibility: {turn:?}"
        );
        assert!(
            matches!(turn.speaker.as_str(), "user" | "host"),
            "specialist role leaked into public transcript: {turn:?}"
        );
    }
    assert!(snapshot.conversation.internal_delegation_count > 0);
    assert!(snapshot
        .conversation
        .internal_activity
        .iter()
        .any(|activity| activity.role == "reviewer"));
}

fn assert_layout_and_image_are_available(snapshot: &CoreRoomSnapshot) {
    let layout = compute_console_layout(snapshot, 220);
    let rail = layout.right_rail.expect("wide layout right rail");
    let sections = rail
        .sections
        .iter()
        .map(|section| section.kind)
        .collect::<Vec<_>>();
    for required in [
        RightRailSectionKind::ProgressWork,
        RightRailSectionKind::Environment,
        RightRailSectionKind::Evidence,
        RightRailSectionKind::Sources,
        RightRailSectionKind::Alerts,
    ] {
        assert!(
            sections.contains(&required),
            "missing right rail section {required:?}; got {sections:?}"
        );
    }

    // The README hero swapped from three synthetic Pillow mockups to
    // a single real terminal capture in v0.9.13. The basic shape of
    // the assertion stays the same: the file is a PNG and has the
    // bulk a real screenshot has.
    let image = include_bytes!("../docs/images/live-room.png");
    assert!(image.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(
        image.len() > 500_000,
        "unexpectedly small README hero image"
    );
}

fn assert_health_signals_keep_negative_cases_visible(snapshot: &CoreRoomSnapshot) {
    let signals = overview_health_signals(snapshot);
    assert!(signals
        .iter()
        .any(|signal| signal.id == "work:blocked:WO-0251"));
    assert!(signals
        .iter()
        .any(|signal| signal.id == "work:stale-tracker:WO-0206"));
    assert!(signals
        .iter()
        .any(|signal| signal.id == "evidence:tracker-stale:WO-0242"));
}

fn assert_transcript_covers_end_to_end_flow() {
    let transcript = include_str!("fixtures/v08_console_dogfood_transcript.txt");
    for required in [
        "User:",
        "Classification: persistent-workorder",
        "WorkOrder: WO-0251",
        "GitHub Issue: #251",
        "Tracker: #238",
        "Required sources:",
        "Role delegation:",
        "Gate phase: signoff",
        "Gate phase: implement",
        "Evidence Packet: WO-0251",
        "Rendered console mock: docs/images/control-room-console.png",
        "Negative fixture: WO-0206",
        "Tracker closure: #238 / #251 / M-AC-10",
        "Remaining v0.9 risks:",
        "Rollback:",
        "Gate phase: closed",
    ] {
        assert!(transcript.contains(required), "missing `{required}`");
    }

    let signoff = transcript
        .find("Gate phase: signoff")
        .expect("signoff phase");
    let implement = transcript
        .find("Gate phase: implement")
        .expect("implement phase");
    assert!(signoff < implement, "signoff must precede implementation");
}

fn assert_readiness_doc_names_v09_risks() {
    let doc = include_str!("../docs/v0.8-console-dogfood.md");
    for required in [
        "v0.9 Readiness Criteria",
        "Remaining v0.9 Risks",
        "CoreRoomSnapshot",
        "public conversation remains `User <-> @host`",
        "The ratatui renderer still needs keyboard focus",
        "The console must not mutate project state directly.",
    ] {
        assert!(
            doc.contains(required),
            "missing readiness doc text `{required}`"
        );
    }
}
