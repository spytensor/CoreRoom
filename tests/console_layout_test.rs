//! Console layout model fixtures.

use coreroom::console_layout::{
    compute_console_layout, pane_priorities, ConsoleBreakpoint, ConsolePaneId, PanePlacement,
    RightRailSectionKind,
};
use coreroom::console_snapshot::CoreRoomSnapshot;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LayoutFixture {
    cases: Vec<LayoutCase>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LayoutCase {
    columns: u16,
    breakpoint: ConsoleBreakpoint,
    conversation_min: u16,
    right_rail_visible: bool,
    visible_panes: Vec<ConsolePaneId>,
    hidden_or_folded_panes: Vec<ConsolePaneId>,
    required_sections: Vec<RightRailSectionKind>,
}

#[test]
fn pane_priorities_keep_public_conversation_first() {
    let priorities = pane_priorities();
    let conversation = priorities
        .iter()
        .find(|pane| pane.pane == ConsolePaneId::PublicConversation)
        .expect("conversation priority");

    assert_eq!(conversation.priority, 0);
    assert!(priorities
        .iter()
        .any(|pane| pane.pane == ConsolePaneId::RightRail));
    assert!(priorities
        .iter()
        .any(|pane| pane.pane == ConsolePaneId::EvidenceClosure));
    assert!(priorities
        .iter()
        .any(|pane| pane.pane == ConsolePaneId::Sources));
}

#[test]
fn layout_width_cases_match_fixture() {
    let snapshot = snapshot();
    let fixture: LayoutFixture =
        toml::from_str(include_str!("fixtures/console_layout_widths.toml")).expect("fixture");

    for case in fixture.cases {
        let layout = compute_console_layout(&snapshot, case.columns);
        assert_eq!(layout.breakpoint, case.breakpoint, "{} cols", case.columns);
        assert!(
            layout.conversation_columns >= case.conversation_min,
            "{} cols: conversation {} < {}",
            case.columns,
            layout.conversation_columns,
            case.conversation_min
        );
        assert_eq!(
            layout.right_rail.is_some(),
            case.right_rail_visible,
            "{} cols",
            case.columns
        );
        for pane in case.visible_panes {
            let decision = pane_decision(&layout, pane);
            assert!(
                decision.visible,
                "{} cols: {pane:?} not visible",
                case.columns
            );
        }
        for pane in case.hidden_or_folded_panes {
            let decision = pane_decision(&layout, pane);
            assert!(
                matches!(
                    decision.placement,
                    PanePlacement::Hidden | PanePlacement::FoldedIntoRightRail
                ),
                "{} cols: {pane:?} placement was {:?}",
                case.columns,
                decision.placement
            );
        }
        if let Some(rail) = layout.right_rail {
            let kinds = rail
                .sections
                .iter()
                .map(|section| section.kind)
                .collect::<Vec<_>>();
            for section in case.required_sections {
                assert!(
                    kinds.contains(&section),
                    "{} cols: missing {section:?}; got {kinds:?}",
                    case.columns
                );
            }
        }
    }
}

#[test]
fn right_rail_contains_effective_project_roles_evidence_sources_and_alerts() {
    let snapshot = snapshot();
    let layout = compute_console_layout(&snapshot, 160);
    let rail = layout.right_rail.expect("right rail");

    assert!(rail
        .sections
        .iter()
        .any(|section| section.kind == RightRailSectionKind::Environment
            && section
                .rows
                .iter()
                .any(|row| row.label == "repo" && row.value == "spytensor/CoreRoom")));
    assert!(rail
        .sections
        .iter()
        .any(|section| section.kind == RightRailSectionKind::Changes
            && section
                .rows
                .iter()
                .any(|row| row.label == "changed files" && row.value == "present")));
    assert!(rail
        .sections
        .iter()
        .any(|section| section.kind == RightRailSectionKind::ActiveRoles
            && section
                .rows
                .iter()
                .any(|row| row.label == "@host" && row.value == "working")));
    assert!(rail
        .sections
        .iter()
        .any(|section| section.kind == RightRailSectionKind::Evidence
            && section.rows.iter().any(|row| row.label == "WO-0242")));
    assert!(rail
        .sections
        .iter()
        .any(|section| section.kind == RightRailSectionKind::Sources
            && section
                .rows
                .iter()
                .any(|row| row.label == "readme-console-mock")));
    assert!(rail
        .sections
        .iter()
        .any(|section| section.kind == RightRailSectionKind::Alerts
            && section
                .rows
                .iter()
                .any(|row| row.source.as_deref() == Some("work:WO-0206"))));
}

#[test]
fn sub_minimum_width_hides_secondary_panes_instead_of_squeezing_conversation() {
    let snapshot = snapshot();
    let layout = compute_console_layout(&snapshot, 100);

    assert_eq!(layout.breakpoint, ConsoleBreakpoint::SubMinimum);
    assert_eq!(layout.conversation_columns, 100);
    assert!(layout.right_rail.is_none());
    assert_eq!(
        pane_decision(&layout, ConsolePaneId::RightRail).placement,
        PanePlacement::Hidden
    );
    assert_eq!(
        pane_decision(&layout, ConsolePaneId::WorkList).placement,
        PanePlacement::Hidden
    );
}

fn pane_decision(
    layout: &coreroom::console_layout::ConsoleLayoutModel,
    pane: ConsolePaneId,
) -> &coreroom::console_layout::PaneDecision {
    layout
        .panes
        .iter()
        .find(|decision| decision.pane == pane)
        .expect("pane decision")
}

fn snapshot() -> CoreRoomSnapshot {
    toml::from_str(include_str!("fixtures/console_snapshot_v08.toml")).expect("snapshot")
}
