//! Terminal render QA fixtures for the v0.9 CoreRoom console.
//!
//! These tests use ratatui's test backend so the supported terminal sizes are
//! checked without requiring a live TTY.

use coreroom::console_actions::route_console_action;
use coreroom::console_navigation::{ConsoleNavigator, ConsoleView};
use coreroom::console_snapshot::CoreRoomSnapshot;
use coreroom::console_tui::{
    render_snapshot_to_text, render_snapshot_to_text_with_action_overlay,
    render_snapshot_to_text_with_nav,
};
use coreroom::host_action::{ActionIntent, HostActionKind, HostActionRequest};

const SUPPORTED_VIEWPORTS: &[(u16, u16)] = &[(80, 30), (120, 40), (160, 48), (220, 54)];

fn snapshot() -> CoreRoomSnapshot {
    toml::from_str(include_str!("fixtures/console_snapshot_v08.toml")).expect("snapshot")
}

#[test]
fn console_terminal_render_fits_supported_viewports() {
    let snapshot = snapshot();

    for &(width, height) in SUPPORTED_VIEWPORTS {
        let rendered = render_snapshot_to_text(&snapshot, width, height).expect("rendered console");
        assert_terminal_fits(&rendered, width);
        assert_nonblank_density(&rendered, width, height);
        assert!(
            rendered.contains("CoreRoom"),
            "missing title at {width}x{height}"
        );
        assert!(
            rendered.contains("Conversation"),
            "missing public conversation panel at {width}x{height}"
        );
        assert!(
            rendered.contains("Public conversation:"),
            "missing public transcript header at {width}x{height}"
        );
    }
}

#[test]
fn console_terminal_active_views_fit_without_polluting_transcript() {
    let snapshot = snapshot();

    for view in [
        ConsoleView::Roles,
        ConsoleView::WorkOrders,
        ConsoleView::Gates,
        ConsoleView::Evidence,
        ConsoleView::Sources,
        ConsoleView::Logs,
        ConsoleView::Xray,
    ] {
        let navigator = ConsoleNavigator {
            active_view: view,
            detail_open: true,
            ..ConsoleNavigator::default()
        };
        let rendered =
            render_snapshot_to_text_with_nav(&snapshot, 160, 48, &navigator).expect("rendered");

        assert_terminal_fits(&rendered, 160);
        assert!(
            rendered.contains(view.label()),
            "missing active view {view:?}"
        );
        assert!(
            !rendered.contains("Public conversation:"),
            "active view {view:?} should not duplicate the public transcript"
        );
    }
}

#[test]
fn console_terminal_preserves_public_transcript_clarity() {
    let snapshot = snapshot();
    let rendered = render_snapshot_to_text(&snapshot, 160, 48).expect("rendered console");

    assert!(rendered.contains("@user <-> @host"));
    assert!(rendered.contains("Internal work:"));
    assert!(rendered.contains("Host-managed task cards"));
    assert!(rendered.contains("@user"));
    assert!(rendered.contains("@host"));
    assert!(rendered.contains("Reviewing snapshot schema without entering public transcript."));
}

#[test]
fn console_terminal_permission_overlay_fits_and_names_host_authority() {
    let snapshot = snapshot();
    let overlay = route_console_action(HostActionRequest::new(
        "HA-261",
        "host",
        "host",
        ActionIntent::Execute,
        HostActionKind::UpdateTracker,
        "WO-0261",
        "Console requests tracker update after terminal QA evidence is collected.",
    ))
    .expect("overlay");

    for &(width, height) in SUPPORTED_VIEWPORTS {
        let rendered = render_snapshot_to_text_with_action_overlay(
            &snapshot,
            width,
            height,
            &ConsoleNavigator::default(),
            &overlay,
        )
        .expect("rendered overlay");

        assert_terminal_fits(&rendered, width);
        assert!(rendered.contains("Host Action"));
        assert!(rendered.contains("CONFIRMATION REQUIRED"));
        assert!(rendered.contains("Action: update-tracker"));
        assert!(rendered.contains("Can execute: false"));
    }
}

fn assert_terminal_fits(rendered: &str, width: u16) {
    let limit = width as usize;
    for (index, line) in rendered.lines().enumerate() {
        let cells = line.chars().count();
        assert!(
            cells <= limit,
            "rendered line {} exceeds width {} with {} cells:\n{}",
            index + 1,
            width,
            cells,
            line
        );
    }
}

fn assert_nonblank_density(rendered: &str, width: u16, height: u16) {
    let nonblank = rendered.chars().filter(|ch| !ch.is_whitespace()).count();
    let area = usize::from(width) * usize::from(height);
    assert!(
        nonblank > area / 80,
        "rendered console is suspiciously sparse at {width}x{height}"
    );
}
