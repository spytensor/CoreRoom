//! Actionable console health selector fixtures.

use coreroom::console_health::{overview_health_signals, select_ids, HealthSelector};
use coreroom::console_snapshot::{CoreRoomSnapshot, HealthSeverity};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HealthFixture {
    expected_selectors: Vec<ExpectedSelector>,
    forbidden_signal_prefixes: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExpectedSelector {
    selector: HealthSelector,
    contains: Vec<String>,
}

#[test]
fn selectors_return_expected_actionable_rows() {
    let snapshot = snapshot();
    let fixture: HealthFixture =
        toml::from_str(include_str!("fixtures/console_health_selectors.toml")).expect("fixture");

    for expected in fixture.expected_selectors {
        let ids = select_ids(&snapshot, expected.selector);
        for item in expected.contains {
            assert!(
                ids.contains(&item),
                "selector {:?} missing {item}; got {ids:?}",
                expected.selector
            );
        }
    }
}

#[test]
fn overview_health_signals_have_severity_citation_and_next_action() {
    let snapshot = snapshot();
    let signals = overview_health_signals(&snapshot);

    assert!(signals
        .iter()
        .any(|signal| signal.id == "work:blocked:WO-0251"));
    assert!(signals
        .iter()
        .any(|signal| signal.id == "work:stale-tracker:WO-0206"));
    assert!(signals
        .iter()
        .any(|signal| signal.id == "role:waiting-approval:qa"));
    assert!(signals
        .iter()
        .any(|signal| signal.id == "source:stale:readme-console-mock"));
    assert!(signals.iter().all(|signal| {
        !signal.observations.is_empty()
            && signal.next_action.as_deref().is_some_and(|s| !s.is_empty())
    }));
    assert!(signals
        .iter()
        .any(|signal| signal.severity == HealthSeverity::Blocking));
}

#[test]
fn non_actionable_informational_metrics_stay_out_of_overview() {
    let snapshot = snapshot();
    let fixture: HealthFixture =
        toml::from_str(include_str!("fixtures/console_health_selectors.toml")).expect("fixture");
    let signals = overview_health_signals(&snapshot);
    let ids = signals
        .iter()
        .map(|signal| signal.id.as_str())
        .collect::<Vec<_>>();

    for prefix in fixture.forbidden_signal_prefixes {
        assert!(
            !ids.iter().any(|id| id.starts_with(&prefix)),
            "non-actionable prefix {prefix} leaked into overview: {ids:?}"
        );
    }
}

fn snapshot() -> CoreRoomSnapshot {
    toml::from_str(include_str!("fixtures/console_snapshot_v08.toml")).expect("snapshot")
}
