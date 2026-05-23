//! Observation, citation, and freshness fixtures.

use coreroom::observation::{
    FreshnessState, Observation, ObservationAuthority, ObservationCitation,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ObservationFixture {
    observations: Vec<Observation>,
}

#[test]
fn observation_fixture_covers_fresh_stale_missing_and_unknown() {
    let fixture: ObservationFixture =
        toml::from_str(include_str!("fixtures/observation_freshness.toml")).expect("parse fixture");

    for observation in &fixture.observations {
        observation.validate().expect("valid observation fixture");
    }

    for required in [
        FreshnessState::Fresh,
        FreshnessState::Stale,
        FreshnessState::Missing,
        FreshnessState::Unknown,
    ] {
        assert!(
            fixture
                .observations
                .iter()
                .any(|observation| observation.freshness.state == required),
            "missing freshness state {required:?}"
        );
    }
}

#[test]
fn github_and_local_validation_citations_are_distinct() {
    let fixture: ObservationFixture =
        toml::from_str(include_str!("fixtures/observation_freshness.toml")).expect("parse fixture");
    let github = fixture
        .observations
        .iter()
        .find(|observation| observation.authority == ObservationAuthority::GitHub)
        .expect("github observation");
    let local = fixture
        .observations
        .iter()
        .find(|observation| observation.authority == ObservationAuthority::LocalValidation)
        .expect("local observation");

    assert!(github
        .citations
        .iter()
        .all(|citation| matches!(citation, ObservationCitation::GitHub { .. })));
    assert!(local
        .citations
        .iter()
        .all(|citation| matches!(citation, ObservationCitation::LocalValidation { .. })));
}

#[test]
fn evidence_claim_without_timestamp_requires_missing_freshness_reason() {
    let observation = Observation {
        id: "obs-bad-evidence-claim".to_owned(),
        summary: "Evidence claim lacks timestamp, commit, source, and missing reason.".to_owned(),
        authority: ObservationAuthority::EvidencePacket,
        freshness: coreroom::observation::ObservationFreshness {
            state: FreshnessState::Unknown,
            observed_at: None,
            max_age_seconds: None,
            missing_reason: Some("No packet timestamp was captured.".to_owned()),
        },
        citations: vec![ObservationCitation::EvidenceClaim {
            work_order: "WO-0243".to_owned(),
            evidence_packet: Some("WO-0243".to_owned()),
            commit: None,
            source: None,
            observed_at: None,
            missing_freshness: None,
        }],
    };

    let err = observation
        .validate()
        .expect_err("missing freshness rejected");
    assert!(err.to_string().contains("evidenceClaim.missingFreshness"));
}

#[test]
fn console_snapshot_alerts_require_observations() {
    let snapshot: coreroom::console_snapshot::CoreRoomSnapshot =
        toml::from_str(include_str!("fixtures/console_snapshot_v08.toml")).expect("snapshot");

    snapshot.validate().expect("snapshot observations valid");
    assert!(snapshot
        .alerts
        .iter()
        .all(|alert| !alert.observations.is_empty()));
}
