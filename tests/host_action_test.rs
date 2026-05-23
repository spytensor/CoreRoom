//! Host action layer fixtures.

use coderoom::host_action::{
    evaluate_host_action, ActionConfirmationRule, ActionConfirmationStatus, ActionOutcome,
    HostActionRequest,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HostActionFixture {
    cases: Vec<HostActionCase>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HostActionCase {
    name: String,
    request: HostActionRequest,
    expected_rule: ActionConfirmationRule,
    expected_outcome: ActionOutcome,
    expected_confirmation: ActionConfirmationStatus,
    expected_can_execute: bool,
    expected_finding: Option<String>,
}

#[test]
fn host_action_fixture_covers_policy_and_safety_cases() {
    let fixture: HostActionFixture =
        toml::from_str(include_str!("fixtures/host_action_layer.toml")).expect("parse fixture");
    let case_names = fixture
        .cases
        .iter()
        .map(|case| case.name.as_str())
        .collect::<Vec<_>>();

    for required in [
        "allowed-classification",
        "confirmation-required-issue-create",
        "confirmed-tracker-update",
        "denied-specialist-execute",
        "blocked-needs-human-input",
        "repeated-action-circuit-breaker",
        "human-only-constitution",
    ] {
        assert!(case_names.contains(&required), "missing {required}");
    }

    for case in fixture.cases {
        let result = evaluate_host_action(case.request).expect("evaluate");
        assert_eq!(result.decision.rule, case.expected_rule, "{}", case.name);
        assert_eq!(
            result.decision.outcome, case.expected_outcome,
            "{}",
            case.name
        );
        assert_eq!(
            result.decision.confirmation_status, case.expected_confirmation,
            "{}",
            case.name
        );
        assert_eq!(
            result.decision.can_execute, case.expected_can_execute,
            "{}",
            case.name
        );
        assert_eq!(result.audit_event.outcome, result.decision.outcome);
        assert_eq!(
            result.audit_event.confirmation_status,
            result.decision.confirmation_status
        );
        assert!(!result.audit_event.rollback_hint.trim().is_empty());
        let summary = result.render_host_summary();
        assert!(summary.contains("HostAction"));
        assert!(summary.contains("outcome:"));

        if let Some(expected) = case.expected_finding {
            assert!(
                result.decision.safety_findings.iter().any(|finding| {
                    finding.code.contains(&expected) || finding.message.contains(&expected)
                }) || result.decision.reason.contains(&expected),
                "case {} missing expected finding `{}` in {:?}",
                case.name,
                expected,
                result.decision
            );
        }
    }
}
