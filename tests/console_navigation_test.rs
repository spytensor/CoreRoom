//! Console navigation state fixtures.

use coreroom::console_layout::ConsoleBreakpoint;
use coreroom::console_navigation::{responsive_state, visible_rows, ConsoleNavigator, ConsoleView};
use coreroom::console_snapshot::CoreRoomSnapshot;
use coreroom::crep::{CrepEvent, TurnOutcome};
use crossterm::event::{KeyCode, KeyModifiers};

fn snapshot() -> CoreRoomSnapshot {
    toml::from_str(include_str!("fixtures/console_snapshot_v08.toml")).expect("snapshot")
}

#[test]
fn navigator_supports_tab_switching_selection_and_detail_state() {
    let mut nav = ConsoleNavigator::default();
    assert_eq!(nav.active_view, ConsoleView::Overview);

    nav.apply_key(KeyCode::Tab, KeyModifiers::NONE, 2);
    assert_eq!(nav.active_view, ConsoleView::Roles);
    assert_eq!(nav.selected, 0);

    nav.apply_key(KeyCode::Down, KeyModifiers::NONE, 3);
    nav.apply_key(KeyCode::Char('j'), KeyModifiers::NONE, 3);
    assert_eq!(nav.selected, 2);

    nav.apply_key(KeyCode::Down, KeyModifiers::NONE, 3);
    assert_eq!(nav.selected, 2);

    nav.apply_key(KeyCode::Enter, KeyModifiers::NONE, 3);
    assert!(nav.detail_open);
    nav.apply_key(KeyCode::Esc, KeyModifiers::NONE, 3);
    assert!(!nav.detail_open);

    nav.apply_key(KeyCode::BackTab, KeyModifiers::SHIFT, 3);
    assert_eq!(nav.active_view, ConsoleView::Overview);
}

#[test]
fn visible_rows_apply_filters_search_and_logs_without_mutating_snapshot() {
    let snapshot = snapshot();
    let mut nav = ConsoleNavigator {
        active_view: ConsoleView::WorkOrders,
        ..ConsoleNavigator::default()
    };
    let all_work = visible_rows(&snapshot, &[], &nav);
    assert!(all_work.iter().any(|row| row.primary == "WO-0242"));

    nav.set_filter(Some("WO-0206".to_owned()), all_work.len());
    let filtered = visible_rows(&snapshot, &[], &nav);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].primary, "WO-0206");

    nav.set_filter(None, filtered.len());
    nav.set_search(Some("CoreRoomSnapshot".to_owned()), all_work.len());
    let searched = visible_rows(&snapshot, &[], &nav);
    assert!(searched.iter().any(|row| row.primary == "WO-0242"));

    nav.active_view = ConsoleView::Logs;
    nav.set_search(Some("permission".to_owned()), 0);
    let logs = visible_rows(&snapshot, &crep_events(), &nav);
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].primary, "permission_denied");
}

#[test]
fn responsive_state_covers_80_120_160_and_220_column_widths() {
    let snapshot = snapshot();

    let eighty = responsive_state(&snapshot, 80);
    assert_eq!(eighty.breakpoint, ConsoleBreakpoint::SubMinimum);
    assert!(!eighty.right_rail_visible);
    assert_eq!(eighty.conversation_columns, 80);

    let compact = responsive_state(&snapshot, 120);
    assert_eq!(compact.breakpoint, ConsoleBreakpoint::Compact120);
    assert!(compact.right_rail_visible);

    let standard = responsive_state(&snapshot, 160);
    assert_eq!(standard.breakpoint, ConsoleBreakpoint::Standard160);
    assert!(standard.conversation_columns >= 104);

    let wide = responsive_state(&snapshot, 220);
    assert_eq!(wide.breakpoint, ConsoleBreakpoint::Wide220);
    assert!(wide.right_rail_visible);
}

fn crep_events() -> Vec<CrepEvent> {
    vec![
        CrepEvent::RoleSpoke {
            role: "host".to_owned(),
            priors_hash: "sha256:host".to_owned(),
            text: "Public summary.".to_owned(),
            mentions: Vec::new(),
            cost_usd: 0.01,
            cache_read: 128,
            turn_id: "turn-host".to_owned(),
            thread_id: "thread-wo-242".to_owned(),
            outcome: TurnOutcome::Converged,
            phase_block: None,
        },
        CrepEvent::PermissionDenied {
            role: "reviewer".to_owned(),
            priors_hash: "sha256:reviewer".to_owned(),
            tool_name: "Bash".to_owned(),
            tool_input: serde_json::json!({"command":"git push --force"}),
            reason: "permission denied".to_owned(),
            turn_id: "turn-reviewer".to_owned(),
            thread_id: "thread-wo-242".to_owned(),
        },
    ]
}
