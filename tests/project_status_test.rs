//! Project status rollup fixtures.

use coderoom::project_status::{
    build_project_status, ProjectStatusInput, ProjectWorkState, ReleaseDecision,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectStatusFixture {
    cases: Vec<ProjectStatusCase>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectStatusCase {
    name: String,
    input: ProjectStatusInput,
    expected_decision: ReleaseDecision,
    expected_release_ready: bool,
    expected_state: ProjectWorkState,
    expected_summary: String,
}

#[test]
fn project_status_fixture_covers_release_rollup_states() {
    let fixture: ProjectStatusFixture =
        toml::from_str(include_str!("fixtures/project_status_rollups.toml"))
            .expect("parse fixture");
    let case_names = fixture
        .cases
        .iter()
        .map(|case| case.name.as_str())
        .collect::<Vec<_>>();

    for required in [
        "healthy-continue-work",
        "at-risk",
        "stale-tracker",
        "failed-ci",
        "blocked-human-input",
        "release-ready",
    ] {
        assert!(case_names.contains(&required), "missing {required}");
    }

    for case in fixture.cases {
        let card = build_project_status(case.input).expect("status card");
        assert_eq!(card.decision, case.expected_decision, "{}", case.name);
        assert_eq!(
            card.release_ready, case.expected_release_ready,
            "{}",
            case.name
        );
        assert!(
            card.work_orders
                .iter()
                .any(|work| work.state == case.expected_state),
            "case {} missing state {:?} in {:?}",
            case.name,
            case.expected_state,
            card.work_orders
        );
        let summary = card.render_host_summary();
        assert!(summary.contains(&case.expected_summary), "{}", case.name);
        assert!(summary.contains("Decision:"));
        assert!(summary.contains("Release ready:"));
        assert!(summary.contains("citations:"));
    }
}
