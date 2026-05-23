//! Observation, citation, and freshness model for trustworthy console signals.
//!
//! Observations explain where a console fact came from, when it was observed,
//! and what authority level it has. They are structural evidence metadata; they
//! do not prove semantic correctness by themselves.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::evidence_packet::EvidenceResult;
use crate::source_graph::SourceGraphFindingKind;
use crate::source_registry::SourceTrustLevel;

/// Authority class for an observation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum ObservationAuthority {
    /// GitHub Issue, PR, check, or commit facts.
    #[serde(rename = "github")]
    GitHub,
    /// Local command/test validation observed in the current checkout.
    LocalValidation,
    /// Structured Evidence Packet facts.
    EvidencePacket,
    /// Source graph or source drift facts.
    SourceGraph,
    /// Generated artifact such as a rendered mock image.
    Generated,
    /// Model-authored prose without external structural evidence.
    ModelProse,
}

impl ObservationAuthority {
    /// Stable label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::GitHub => "github",
            Self::LocalValidation => "local-validation",
            Self::EvidencePacket => "evidence-packet",
            Self::SourceGraph => "source-graph",
            Self::Generated => "generated",
            Self::ModelProse => "model-prose",
        }
    }
}

/// Freshness state for an observation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum FreshnessState {
    /// Observation is within the expected freshness window.
    Fresh,
    /// Observation exists but is older than the expected freshness window.
    Stale,
    /// Observation could not be obtained.
    Missing,
    /// Observation freshness is explicitly unknown.
    Unknown,
}

impl FreshnessState {
    /// Stable label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Stale => "stale",
            Self::Missing => "missing",
            Self::Unknown => "unknown",
        }
    }
}

/// Freshness metadata shared by console observations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ObservationFreshness {
    /// Freshness state.
    pub state: FreshnessState,
    /// Timestamp when the fact was observed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<String>,
    /// Expected freshness window in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_age_seconds: Option<u64>,
    /// Why freshness is missing or unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing_reason: Option<String>,
}

impl ObservationFreshness {
    /// Validate freshness metadata.
    pub fn validate(&self) -> Result<()> {
        match self.state {
            FreshnessState::Fresh | FreshnessState::Stale => {
                ensure_nonempty_option("freshness.observedAt", self.observed_at.as_deref())?;
            }
            FreshnessState::Missing | FreshnessState::Unknown => {
                ensure_nonempty_option("freshness.missingReason", self.missing_reason.as_deref())?;
            }
        }
        if self.max_age_seconds == Some(0) {
            bail!("freshness.maxAgeSeconds must be greater than zero");
        }
        Ok(())
    }
}

/// One explainable observation behind a console row or health signal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Observation {
    /// Stable observation id.
    pub id: String,
    /// Human-facing summary.
    pub summary: String,
    /// Observation authority.
    pub authority: ObservationAuthority,
    /// Freshness metadata.
    pub freshness: ObservationFreshness,
    /// Structural citations backing the observation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub citations: Vec<ObservationCitation>,
}

impl Observation {
    /// Validate observation shape.
    pub fn validate(&self) -> Result<()> {
        ensure_nonempty("observation.id", &self.id)?;
        ensure_nonempty("observation.summary", &self.summary)?;
        self.freshness.validate()?;
        if self.citations.is_empty() {
            bail!(
                "observation `{}` must include at least one citation",
                self.id
            );
        }
        for citation in &self.citations {
            citation.validate()?;
        }
        if !self
            .citations
            .iter()
            .any(|citation| citation.authority() == self.authority)
        {
            bail!(
                "observation `{}` authority `{}` is not backed by a matching citation",
                self.id,
                self.authority.label()
            );
        }
        Ok(())
    }
}

/// Structural citation behind an observation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum ObservationCitation {
    /// GitHub facts observed from an issue, PR, check, or head SHA.
    #[serde(rename = "github")]
    GitHub {
        /// GitHub Issue number.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        issue: Option<u64>,
        /// GitHub Pull Request number.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pull_request: Option<u64>,
        /// GitHub check URL.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        check_url: Option<String>,
        /// Git head SHA.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        head_sha: Option<String>,
        /// Timestamp when GitHub facts were observed.
        observed_at: String,
    },
    /// Local command/test validation citation.
    LocalValidation {
        /// Command that produced the local fact.
        command: String,
        /// Command/test result.
        result: EvidenceResult,
        /// Local runner label.
        runner: String,
        /// Timestamp when local validation was observed.
        observed_at: String,
    },
    /// Evidence Packet claim citation.
    EvidenceClaim {
        /// WorkOrder id.
        work_order: String,
        /// Evidence Packet id/path.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        evidence_packet: Option<String>,
        /// Commit SHA associated with the claim.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        commit: Option<String>,
        /// Source label or path associated with the claim.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<String>,
        /// Timestamp when evidence was observed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        observed_at: Option<String>,
        /// Why evidence freshness is missing.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        missing_freshness: Option<String>,
    },
    /// Source graph drift citation.
    SourceDrift {
        /// Source id.
        source_id: String,
        /// Pinned identity.
        pin: String,
        /// Drift finding kind.
        finding_kind: SourceGraphFindingKind,
        /// Source trust level.
        trust_level: SourceTrustLevel,
        /// Timestamp when source facts were observed.
        observed_at: String,
    },
    /// Generated artifact citation.
    Generated {
        /// Artifact path or id.
        artifact: String,
        /// Generation source.
        source: String,
        /// Timestamp when generated output was observed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        observed_at: Option<String>,
        /// Why generated artifact freshness is missing.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        missing_freshness: Option<String>,
    },
    /// Explicitly low-authority model prose citation.
    ModelProse {
        /// Transcript or turn reference.
        reference: String,
        /// Why this prose is insufficient as completion evidence.
        caveat: String,
    },
}

impl ObservationCitation {
    /// Authority class implied by the citation kind.
    pub const fn authority(&self) -> ObservationAuthority {
        match self {
            Self::GitHub { .. } => ObservationAuthority::GitHub,
            Self::LocalValidation { .. } => ObservationAuthority::LocalValidation,
            Self::EvidenceClaim { .. } => ObservationAuthority::EvidencePacket,
            Self::SourceDrift { .. } => ObservationAuthority::SourceGraph,
            Self::Generated { .. } => ObservationAuthority::Generated,
            Self::ModelProse { .. } => ObservationAuthority::ModelProse,
        }
    }

    /// Validate citation shape.
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::GitHub {
                issue,
                pull_request,
                check_url,
                head_sha,
                observed_at,
            } => {
                if issue.is_none()
                    && pull_request.is_none()
                    && check_url.as_deref().is_none_or(str::is_empty)
                    && head_sha.as_deref().is_none_or(str::is_empty)
                {
                    bail!("github citation must cite issue, PR, check URL, or head SHA");
                }
                if issue == &Some(0) {
                    bail!("github citation issue must be non-zero");
                }
                if pull_request == &Some(0) {
                    bail!("github citation pullRequest must be non-zero");
                }
                ensure_nonempty("github.observedAt", observed_at)
            }
            Self::LocalValidation {
                command,
                runner,
                observed_at,
                ..
            } => {
                ensure_nonempty("localValidation.command", command)?;
                ensure_nonempty("localValidation.runner", runner)?;
                ensure_nonempty("localValidation.observedAt", observed_at)
            }
            Self::EvidenceClaim {
                work_order,
                evidence_packet,
                commit,
                source,
                observed_at,
                missing_freshness,
            } => {
                ensure_work_order_id(work_order)?;
                if evidence_packet.as_deref().is_some_and(str::is_empty) {
                    bail!("evidence claim evidencePacket cannot be empty");
                }
                let has_structural_freshness =
                    commit.as_deref().is_some_and(|value| !value.is_empty())
                        || source.as_deref().is_some_and(|value| !value.is_empty())
                        || observed_at
                            .as_deref()
                            .is_some_and(|value| !value.is_empty());
                if !has_structural_freshness {
                    ensure_nonempty_option(
                        "evidenceClaim.missingFreshness",
                        missing_freshness.as_deref(),
                    )?;
                }
                Ok(())
            }
            Self::SourceDrift {
                source_id,
                pin,
                observed_at,
                ..
            } => {
                ensure_nonempty("sourceDrift.sourceId", source_id)?;
                ensure_nonempty("sourceDrift.pin", pin)?;
                ensure_nonempty("sourceDrift.observedAt", observed_at)
            }
            Self::Generated {
                artifact,
                source,
                observed_at,
                missing_freshness,
            } => {
                ensure_nonempty("generated.artifact", artifact)?;
                ensure_nonempty("generated.source", source)?;
                if observed_at.as_deref().is_none_or(str::is_empty) {
                    ensure_nonempty_option(
                        "generated.missingFreshness",
                        missing_freshness.as_deref(),
                    )?;
                }
                Ok(())
            }
            Self::ModelProse { reference, caveat } => {
                ensure_nonempty("modelProse.reference", reference)?;
                ensure_nonempty("modelProse.caveat", caveat)
            }
        }
    }
}

fn ensure_nonempty(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{field} cannot be empty");
    }
    Ok(())
}

fn ensure_nonempty_option(field: &str, value: Option<&str>) -> Result<()> {
    match value {
        Some(value) => ensure_nonempty(field, value),
        None => bail!("{field} cannot be empty"),
    }
}

fn ensure_work_order_id(value: &str) -> Result<()> {
    ensure_nonempty("workOrder", value)?;
    if !value.starts_with("WO-") {
        bail!("WorkOrder id `{value}` must start with `WO-`");
    }
    Ok(())
}
