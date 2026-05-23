//! GitHub-native WorkOrder lifecycle fixtures.

use coreroom::github_status::{
    derive_github_work_order_status, GitHubWorkOrderFacts, WorkOrderLifecycle,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LifecycleFixture {
    cases: Vec<LifecycleCase>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LifecycleCase {
    name: String,
    expected_lifecycle: WorkOrderLifecycle,
    facts: GitHubWorkOrderFacts,
    expected_finding: Option<String>,
}

#[test]
fn github_status_fixture_covers_v07_lifecycle_states() {
    let fixture: LifecycleFixture =
        toml::from_str(include_str!("fixtures/github_workorder_statuses.toml"))
            .expect("parse fixture");
    let case_names = fixture
        .cases
        .iter()
        .map(|case| case.name.as_str())
        .collect::<Vec<_>>();

    for required in [
        "not-started",
        "ready",
        "in-progress",
        "in-review",
        "failed-ci",
        "merged-tracker-stale",
        "blocked",
        "fully-closed",
        "failed-ci-tracker-stale",
    ] {
        assert!(case_names.contains(&required), "missing {required}");
    }

    for case in fixture.cases {
        let report = derive_github_work_order_status(&case.facts);
        assert_eq!(
            report.lifecycle, case.expected_lifecycle,
            "case {}",
            case.name
        );
        let summary = report.render_host_summary(&case.facts);
        assert!(summary.contains(&format!("#{}", case.facts.issue)));
        assert!(summary.contains("tracker:"));
        assert!(summary.contains("evidence:"));
        if let Some(expected) = case.expected_finding {
            assert!(
                report
                    .findings
                    .iter()
                    .any(|finding| finding.contains(&expected)),
                "case {} missing finding containing `{}` in {:?}",
                case.name,
                expected,
                report.findings
            );
        }
    }
}
