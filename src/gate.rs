//! SDLC gate ledgers, artifact validation, and completion guards.
//!
//! The gate layer is intentionally structural. It can prove that required
//! evidence is present, named, and linked to local files; it does not claim
//! an implementation is semantically correct.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::adapter::Engine;
use crate::config::{AuthorityScope, Config, COREROOM_DIR};
use crate::manifest::sha256_file;

/// Subdirectory inside `.coreroom/` that stores per-thread gate ledgers.
pub const GATES_DIR: &str = "gates";

/// Subdirectory inside `.coreroom/` that stores reusable SDLC gate templates.
pub const GATE_TEMPLATES_DIR: &str = "gate-templates";

const ACTIVE_GATE_FILE: &str = "active";
const LEDGER_SCHEMA_VERSION: u32 = 1;

const TIER_CLASSIFY_TEMPLATE: &str = include_str!("gate_templates/tier-classify.md");
const RESEARCH_GATE_TEMPLATE: &str = include_str!("gate_templates/research-gate.md");
const PLAN_GATE_TEMPLATE: &str = include_str!("gate_templates/plan-gate.md");
const PLAN_REVIEW_GATE_TEMPLATE: &str = include_str!("gate_templates/plan-review-gate.md");
const CODE_REVIEW_GATE_TEMPLATE: &str = include_str!("gate_templates/code-review-gate.md");
const PRECOMMIT_GATE_TEMPLATE: &str = include_str!("gate_templates/precommit-gate.md");
const SIGNOFF_GATE_TEMPLATE: &str = include_str!("gate_templates/signoff-gate.md");

/// Built-in SDLC gate template asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateTemplate {
    /// File name written under `.coreroom/gate-templates/`.
    pub filename: &'static str,
    /// Template body.
    pub content: &'static str,
}

/// Return the built-in gate templates written for new projects.
#[must_use]
pub const fn default_templates() -> &'static [GateTemplate] {
    &[
        GateTemplate {
            filename: "tier-classify.md",
            content: TIER_CLASSIFY_TEMPLATE,
        },
        GateTemplate {
            filename: "research-gate.md",
            content: RESEARCH_GATE_TEMPLATE,
        },
        GateTemplate {
            filename: "plan-gate.md",
            content: PLAN_GATE_TEMPLATE,
        },
        GateTemplate {
            filename: "plan-review-gate.md",
            content: PLAN_REVIEW_GATE_TEMPLATE,
        },
        GateTemplate {
            filename: "code-review-gate.md",
            content: CODE_REVIEW_GATE_TEMPLATE,
        },
        GateTemplate {
            filename: "precommit-gate.md",
            content: PRECOMMIT_GATE_TEMPLATE,
        },
        GateTemplate {
            filename: "signoff-gate.md",
            content: SIGNOFF_GATE_TEMPLATE,
        },
    ]
}

/// SDLC tier for a work item.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GateTier {
    /// Lightweight work. No cross-model review requirement by default.
    Tier0,
    /// Code-changing or risk-bearing work that requires SDLC evidence.
    Tier1,
}

impl GateTier {
    /// Parse a user-facing tier token.
    pub fn parse(input: &str) -> Result<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "0" | "tier0" | "tier-0" | "tier_0" => Ok(Self::Tier0),
            "1" | "tier1" | "tier-1" | "tier_1" => Ok(Self::Tier1),
            other => bail!("unknown tier `{other}`; use 0 or 1"),
        }
    }

    /// Compact label used in CLI output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Tier0 => "Tier 0",
            Self::Tier1 => "Tier 1",
        }
    }
}

/// SDLC phase tracked by a ledger.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GatePhase {
    /// Initial intake and work classification.
    Intake,
    /// Discovery and research before planning.
    #[serde(alias = "research")]
    Discovery,
    /// Plan creation before review.
    Plan,
    /// Peer, authority, or cross-model review is in progress.
    Review,
    /// Sign-off evidence is being collected.
    Signoff,
    /// Implementation is in progress.
    #[serde(alias = "implementation")]
    Implement,
    /// QA / verification is in progress.
    #[serde(alias = "precommit")]
    Qa,
    /// Gate is closed.
    Closed,
    /// Gate was rejected from review or signoff.
    Rejected,
}

impl GatePhase {
    /// Linear workflow phases.
    pub const LINEAR: [Self; 8] = [
        Self::Intake,
        Self::Discovery,
        Self::Plan,
        Self::Review,
        Self::Signoff,
        Self::Implement,
        Self::Qa,
        Self::Closed,
    ];

    /// Parse a user-facing phase token.
    pub fn parse(input: &str) -> Result<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "intake" => Ok(Self::Intake),
            "discovery" | "discover" | "research" => Ok(Self::Discovery),
            "plan" => Ok(Self::Plan),
            "review" => Ok(Self::Review),
            "signoff" | "sign-off" => Ok(Self::Signoff),
            "implement" | "implementation" | "impl" => Ok(Self::Implement),
            "qa" | "verify" | "verification" | "precommit" | "pre-commit" => Ok(Self::Qa),
            "closed" | "close" => Ok(Self::Closed),
            "rejected" | "reject" => Ok(Self::Rejected),
            other => bail!("unknown phase `{other}`"),
        }
    }

    /// Compact label used in CLI output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Intake => "intake",
            Self::Discovery => "discovery",
            Self::Plan => "plan",
            Self::Review => "review",
            Self::Signoff => "signoff",
            Self::Implement => "implement",
            Self::Qa => "qa",
            Self::Closed => "closed",
            Self::Rejected => "rejected",
        }
    }

    /// Return the next legal forward phase, if any.
    #[must_use]
    pub fn next(self) -> Option<Self> {
        Self::LINEAR
            .iter()
            .position(|phase| *phase == self)
            .and_then(|index| Self::LINEAR.get(index + 1).copied())
    }

    /// Whether `to` is a legal non-rollback transition from this phase.
    #[must_use]
    pub fn can_advance_to(self, to: Self) -> bool {
        self.next() == Some(to)
            || matches!((self, to), (Self::Review | Self::Signoff, Self::Rejected))
    }

    /// Whether `to` is an earlier linear phase and therefore a legal rollback.
    #[must_use]
    pub fn can_rollback_to(self, to: Self) -> bool {
        let from = Self::linear_index(self);
        let to = Self::linear_index(to);
        matches!((from, to), (Some(from), Some(to)) if to < from)
    }

    fn linear_index(self) -> Option<usize> {
        Self::LINEAR.iter().position(|phase| *phase == self)
    }
}

/// Structural validation result.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GateResult {
    /// All configured structural checks pass.
    Pass,
    /// Evidence is present but structurally invalid.
    Fail,
    /// Required evidence is missing.
    Incomplete,
    /// Gate was explicitly bypassed with a reason.
    Bypassed,
}

impl GateResult {
    /// Compact label used in CLI output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Incomplete => "incomplete",
            Self::Bypassed => "bypassed",
        }
    }
}

/// Artifact type recorded in the ledger.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum GateArtifactKind {
    /// Discovery artifact.
    #[serde(alias = "research")]
    Discovery,
    /// Implementation plan artifact.
    Plan,
    /// Plan review artifact.
    PlanReview,
    /// Code review artifact.
    Review,
    /// QA / verification artifact.
    #[serde(alias = "precommit")]
    Qa,
    /// Sign-off artifact.
    Signoff,
}

impl GateArtifactKind {
    /// Parse a user-facing artifact kind.
    pub fn parse(input: &str) -> Result<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "discovery" | "research" => Ok(Self::Discovery),
            "plan" => Ok(Self::Plan),
            "plan-review" | "plan_review" | "planreview" => Ok(Self::PlanReview),
            "review" | "code-review" | "code_review" => Ok(Self::Review),
            "qa" | "verification" | "precommit" | "pre-commit" => Ok(Self::Qa),
            "signoff" | "sign-off" => Ok(Self::Signoff),
            other => bail!("unknown artifact kind `{other}`"),
        }
    }

    /// Compact label used in CLI output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Discovery => "discovery",
            Self::Plan => "plan",
            Self::PlanReview => "plan-review",
            Self::Review => "review",
            Self::Qa => "qa",
            Self::Signoff => "signoff",
        }
    }
}

/// Actor metadata for implementers and reviewers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GateActor {
    /// CoreRoom role name.
    pub role: String,
    /// Engine used by that role.
    pub engine: Engine,
    /// Engine model identifier.
    pub model: String,
    /// CoreRoom turn id, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    /// CoreRoom thread id, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

/// Recorded artifact path and attribution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GateArtifact {
    /// Artifact kind.
    pub kind: GateArtifactKind,
    /// Path relative to the project root, or an absolute path.
    pub path: String,
    /// Producing role, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// CoreRoom turn id, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    /// Creation timestamp.
    pub created_at: String,
}

/// Recorded review turn and structural review summary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GateReview {
    /// Reviewer role and model metadata.
    pub reviewer: GateActor,
    /// Whether this is the same role as the implementer.
    pub same_role_as_implementer: bool,
    /// Number of blocking findings reported by the review artifact.
    pub blocking_count: u32,
    /// Number of warning findings reported by the review artifact.
    pub warning_count: u32,
    /// Whether the review includes file:line evidence.
    pub file_line_evidence: bool,
    /// Whether all blocking findings have been resolved.
    pub all_blockings_resolved: bool,
    /// Review artifact path, when recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_path: Option<String>,
    /// Creation timestamp.
    pub created_at: String,
}

/// Binding plan review decision recorded by an authority-scoped role.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PlanReviewDecision {
    /// Role approves the current plan SHA for its intersecting scopes.
    Approve,
    /// Role rejects the current plan SHA. Blocks signoff until override or re-review.
    Reject,
    /// Role asks for changes before approval. Blocks signoff until re-review.
    NeedsRevision,
}

impl PlanReviewDecision {
    /// Parse a user-facing decision token.
    pub fn parse(input: &str) -> Result<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "approve" | "approved" => Ok(Self::Approve),
            "reject" | "rejected" => Ok(Self::Reject),
            "needs-revision" | "needs_revision" | "needsrevision" => Ok(Self::NeedsRevision),
            other => bail!(
                "unknown plan review decision `{other}`; use approve, reject, or needs-revision"
            ),
        }
    }

    /// Compact label for CLI output and CREP.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::Reject => "reject",
            Self::NeedsRevision => "needs-revision",
        }
    }

    fn blocks_signoff(self) -> bool {
        matches!(self, Self::Reject | Self::NeedsRevision)
    }
}

/// Persisted authority-scoped plan review.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GateRoleReviewRecord {
    /// Review decision.
    pub decision: PlanReviewDecision,
    /// Human-readable reason for reject/needs-revision, or optional approval note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Reviewer role and model metadata resolved from config.
    pub reviewer: GateActor,
    /// SHA-256 of the plan artifact content at review time.
    pub plan_sha: String,
    /// Plan artifact path reviewed.
    pub plan_path: String,
    /// Plan scopes this role reviewed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<AuthorityScope>,
    /// Creation timestamp.
    pub created_at: String,
}

/// Explicit user override for a blocking authority review.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GatePlanOverride {
    /// Role whose blocking review is overruled.
    pub role: String,
    /// User-provided reason.
    pub reason: String,
    /// Plan SHA the override applies to.
    pub plan_sha: String,
    /// Creation timestamp.
    pub created_at: String,
}

/// Recorded verification command or cited evidence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GateVerification {
    /// Command or verification method.
    pub command: String,
    /// Whether the verification passed.
    pub ok: bool,
    /// Evidence snippet or command output.
    pub evidence: String,
    /// Creation timestamp.
    pub created_at: String,
}

/// Explicit accepted risk or bypass entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GateBypass {
    /// Gate or rule that was bypassed.
    pub gate: String,
    /// Human-readable reason.
    pub reason: String,
    /// Blocking reasons present when the bypass was recorded.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_reasons: Vec<String>,
    /// Creation timestamp.
    pub created_at: String,
}

/// A role-declared phase block recorded on a gate ledger.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GatePhaseBlock {
    /// Phase that was blocked.
    pub phase: GatePhase,
    /// Role that declared the block.
    pub role: String,
    /// Human-readable reason from the role.
    pub reason: String,
    /// Creation timestamp.
    pub created_at: String,
}

/// Append-only note in a gate ledger.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GateHistoryEntry {
    /// Timestamp for this history entry.
    pub at: String,
    /// Event name.
    pub event: String,
    /// Human-readable detail.
    pub detail: String,
}

/// Persistent SDLC gate ledger stored under `.coreroom/gates/`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GateLedger {
    /// Schema version.
    pub schema_version: u32,
    /// CoreRoom thread id or user-provided work id.
    pub thread_id: String,
    /// Work item title.
    pub feature: String,
    /// SDLC tier.
    pub tier: GateTier,
    /// Current SDLC phase.
    pub phase: GatePhase,
    /// Implementing role metadata, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implementer: Option<GateActor>,
    /// Recorded evidence artifacts.
    #[serde(default)]
    pub artifacts: Vec<GateArtifact>,
    /// Recorded review turns.
    #[serde(default)]
    pub reviewers: Vec<GateReview>,
    /// Binding authority-scoped plan review decisions.
    #[serde(default)]
    pub role_reviews: Vec<GateRoleReviewRecord>,
    /// User overrides for binding plan review blocks.
    #[serde(default)]
    pub plan_overrides: Vec<GatePlanOverride>,
    /// Recorded verification evidence.
    #[serde(default)]
    pub verifications: Vec<GateVerification>,
    /// Explicit accepted risks and bypasses.
    #[serde(default)]
    pub bypasses: Vec<GateBypass>,
    /// Role-declared phase blocks.
    #[serde(default)]
    pub phase_blocks: Vec<GatePhaseBlock>,
    /// Current structural gate result.
    pub result: GateResult,
    /// Creation timestamp.
    pub created_at: String,
    /// Last update timestamp.
    pub updated_at: String,
    /// Ledger history.
    #[serde(default)]
    pub history: Vec<GateHistoryEntry>,
}

/// Parameters for creating a gate ledger.
#[derive(Debug, Clone)]
pub struct GateInit {
    /// Thread id.
    pub thread_id: String,
    /// Work item title.
    pub feature: String,
    /// SDLC tier.
    pub tier: GateTier,
    /// Initial phase.
    pub phase: GatePhase,
    /// Optional implementer metadata.
    pub implementer: Option<GateActor>,
}

/// Parameters for recording an artifact.
#[derive(Debug, Clone)]
pub struct ArtifactInput {
    /// Thread id.
    pub thread_id: String,
    /// Artifact kind.
    pub kind: GateArtifactKind,
    /// Artifact path.
    pub path: String,
    /// Producing role.
    pub role: Option<String>,
    /// Producing turn id.
    pub turn_id: Option<String>,
}

/// Parameters for recording a review.
#[derive(Debug, Clone)]
pub struct ReviewInput {
    /// Thread id.
    pub thread_id: String,
    /// Reviewer metadata.
    pub reviewer: GateActor,
    /// Whether the reviewer is the implementer role.
    pub same_role_as_implementer: bool,
    /// Blocking finding count.
    pub blocking_count: u32,
    /// Warning finding count.
    pub warning_count: u32,
    /// Whether the review includes file:line evidence.
    pub file_line_evidence: bool,
    /// Whether blockings are resolved.
    pub all_blockings_resolved: bool,
    /// Review artifact path.
    pub artifact_path: Option<String>,
}

/// Parameters for recording verification evidence.
#[derive(Debug, Clone)]
pub struct VerificationInput {
    /// Thread id.
    pub thread_id: String,
    /// Command or method.
    pub command: String,
    /// Whether it passed.
    pub ok: bool,
    /// Evidence text.
    pub evidence: String,
}

/// Parameters for recording an authority-scoped plan review.
#[derive(Debug, Clone)]
pub struct RoleReviewInput {
    /// Thread id.
    pub thread_id: String,
    /// Reviewer role.
    pub role: String,
    /// Review decision.
    pub decision: PlanReviewDecision,
    /// Optional reason.
    pub reason: Option<String>,
}

/// Parameters for overriding an authority-scoped plan review block.
#[derive(Debug, Clone)]
pub struct PlanOverrideInput {
    /// Thread id.
    pub thread_id: String,
    /// Role whose block is overruled.
    pub role: String,
    /// User-provided reason.
    pub reason: String,
}

/// Parameters for an explicit phase transition.
#[derive(Debug, Clone)]
pub struct PhaseAdvanceInput {
    /// Thread id.
    pub thread_id: String,
    /// Target phase.
    pub to: GatePhase,
    /// Actor responsible for the transition.
    pub actor: String,
    /// Rollback justification. When present, target must be an earlier phase.
    pub rollback_reason: Option<String>,
}

/// Result of a phase transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhaseTransition {
    /// Thread id.
    pub thread_id: String,
    /// Previous phase.
    pub from: GatePhase,
    /// New phase.
    pub to: GatePhase,
    /// Actor responsible for the transition.
    pub actor: String,
    /// Rollback justification, when this was a rollback.
    pub rollback_reason: Option<String>,
}

/// Structural validation output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateValidation {
    /// Ledger thread id.
    pub thread_id: String,
    /// SDLC tier.
    pub tier: GateTier,
    /// Validation result.
    pub result: GateResult,
    /// Blocking reasons.
    pub reasons: Vec<String>,
    /// Non-blocking warnings.
    pub warnings: Vec<String>,
    /// Current authority review status, when a plan artifact is present.
    pub plan_review_status: Option<PlanReviewStatus>,
}

impl GateValidation {
    /// Whether the validation passed.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.result == GateResult::Pass
    }
}

/// Status of the current plan's authority-scoped reviews.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanReviewStatus {
    /// Plan artifact path.
    pub plan_path: String,
    /// Current plan SHA.
    pub plan_sha: String,
    /// Scopes declared by the plan artifact.
    pub scopes: Vec<AuthorityScope>,
    /// Required reviewer statuses.
    pub required: Vec<PlanReviewerStatus>,
    /// Plan scopes that are partially uncovered after at least one match exists.
    pub uncovered_scopes: Vec<AuthorityScope>,
    /// Non-blocking status warnings.
    pub warnings: Vec<String>,
}

/// One required authority reviewer's status for the current plan SHA.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanReviewerStatus {
    /// Role name.
    pub role: String,
    /// Intersecting authority scopes.
    pub scopes: Vec<AuthorityScope>,
    /// Latest recorded decision, if any.
    pub decision: Option<PlanReviewDecision>,
    /// Decision reason, if any.
    pub reason: Option<String>,
    /// Whether the latest decision was for an older plan SHA.
    pub stale: bool,
    /// Whether a user override covers this role for the current plan SHA.
    pub overridden: bool,
}

impl PlanReviewStatus {
    fn blocking_reasons(&self) -> Vec<String> {
        let mut reasons = Vec::new();
        if !self.uncovered_scopes.is_empty() {
            reasons.push(format!(
                "plan scopes lack authority coverage: {}",
                format_scopes(&self.uncovered_scopes)
            ));
        }
        for reviewer in &self.required {
            if reviewer.overridden {
                continue;
            }
            match reviewer.decision {
                None => reasons.push(format!("authority review missing from @{}", reviewer.role)),
                Some(_) if reviewer.stale => reasons.push(format!(
                    "authority review from @{} is stale for current plan SHA",
                    reviewer.role
                )),
                Some(PlanReviewDecision::Approve) => {}
                Some(decision) if decision.blocks_signoff() => {
                    let reason = reviewer
                        .reason
                        .as_deref()
                        .map_or(String::new(), |reason| format!(": {reason}"));
                    reasons.push(format!(
                        "authority review from @{} is {}{reason}",
                        reviewer.role,
                        decision.label()
                    ));
                }
                Some(_) => {}
            }
        }
        reasons
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlanArtifactInfo {
    path: String,
    sha: String,
    scopes: Vec<AuthorityScope>,
}

/// Outcome of installing gate templates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemplateInstallOutcome {
    /// Files written.
    pub written: usize,
    /// Existing files skipped.
    pub skipped: usize,
}

/// Install default SDLC gate templates under `.coreroom/gate-templates/`.
pub fn install_templates(coreroom_dir: &Path, overwrite: bool) -> Result<TemplateInstallOutcome> {
    let dir = coreroom_dir.join(GATE_TEMPLATES_DIR);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let mut written = 0usize;
    let mut skipped = 0usize;
    for template in default_templates() {
        let path = dir.join(template.filename);
        if path.exists() && !overwrite {
            skipped += 1;
            continue;
        }
        std::fs::write(&path, ensure_trailing_newline(template.content))
            .with_context(|| format!("writing {}", path.display()))?;
        written += 1;
    }
    Ok(TemplateInstallOutcome { written, skipped })
}

/// Create or replace a gate ledger and mark it active.
pub fn init(project_root: &Path, input: GateInit) -> Result<GateLedger> {
    if input.thread_id.trim().is_empty() {
        bail!("thread id cannot be empty");
    }
    if input.feature.trim().is_empty() {
        bail!("feature cannot be empty");
    }
    let coreroom_dir = project_root.join(COREROOM_DIR);
    ensure_gate_dirs(&coreroom_dir)?;
    let now = now_string();
    let mut ledger = GateLedger {
        schema_version: LEDGER_SCHEMA_VERSION,
        thread_id: input.thread_id.trim().to_owned(),
        feature: input.feature.trim().to_owned(),
        tier: input.tier,
        phase: input.phase,
        implementer: input.implementer,
        artifacts: Vec::new(),
        reviewers: Vec::new(),
        role_reviews: Vec::new(),
        plan_overrides: Vec::new(),
        verifications: Vec::new(),
        bypasses: Vec::new(),
        phase_blocks: Vec::new(),
        result: GateResult::Incomplete,
        created_at: now.clone(),
        updated_at: now,
        history: Vec::new(),
    };
    ledger.push_history(
        "created",
        format!(
            "{} gate created for {}",
            ledger.tier.label(),
            ledger.feature
        ),
    );
    save_ledger(&coreroom_dir, &ledger)?;
    ensure_phase_artifact(
        project_root,
        &ledger,
        ledger.phase,
        ledger.phase,
        "init",
        None,
    )?;
    write_active_thread(&coreroom_dir, &ledger.thread_id)?;
    Ok(ledger)
}

/// Load a selected ledger, defaulting to the active gate.
pub fn load(project_root: &Path, thread_id: Option<&str>) -> Result<GateLedger> {
    let coreroom_dir = project_root.join(COREROOM_DIR);
    let thread_id = selected_thread_id(&coreroom_dir, thread_id)?;
    load_ledger(&coreroom_dir, &thread_id)
}

/// Record implementer metadata on an existing ledger.
pub fn set_implementer(
    project_root: &Path,
    thread_id: &str,
    actor: GateActor,
) -> Result<GateLedger> {
    update_ledger(project_root, thread_id, |ledger| {
        ledger.implementer = Some(actor);
        ledger.push_history("implementer", "implementer metadata recorded");
    })
}

fn ensure_evidence_writes_allowed(
    project_root: &Path,
    thread_id: &str,
    operation: &str,
) -> Result<()> {
    let ledger = load(project_root, Some(thread_id))?;
    if ledger.tier == GateTier::Tier0 {
        bail!(
            "Tier 0/read-only gates do not record {operation}; report inline evidence or rerun the work as Tier 1 with explicit user approval"
        );
    }
    Ok(())
}

/// Record an artifact path on an existing ledger.
pub fn record_artifact(project_root: &Path, input: ArtifactInput) -> Result<GateLedger> {
    let path = input.path.trim();
    if path.is_empty() {
        bail!("artifact path cannot be empty");
    }
    let thread_id = input.thread_id.clone();
    ensure_evidence_writes_allowed(project_root, &thread_id, "gate artifacts")?;
    update_ledger(project_root, &thread_id, |ledger| {
        let detail = format!("{} artifact recorded at {path}", input.kind.label());
        ledger.artifacts.push(GateArtifact {
            kind: input.kind,
            path: path.to_owned(),
            role: input.role,
            turn_id: input.turn_id,
            created_at: now_string(),
        });
        ledger.push_history("artifact", detail);
    })
}

/// Record a review turn on an existing ledger.
pub fn record_review(project_root: &Path, input: ReviewInput) -> Result<GateLedger> {
    let thread_id = input.thread_id.clone();
    ensure_evidence_writes_allowed(project_root, &thread_id, "review evidence")?;
    update_ledger(project_root, &thread_id, |ledger| {
        let role = input.reviewer.role.clone();
        let engine = input.reviewer.engine.as_str();
        let model = input.reviewer.model.clone();
        if let Some(path) = &input.artifact_path {
            ledger.artifacts.push(GateArtifact {
                kind: GateArtifactKind::Review,
                path: path.clone(),
                role: Some(role.clone()),
                turn_id: input.reviewer.turn_id.clone(),
                created_at: now_string(),
            });
        }
        ledger.reviewers.push(GateReview {
            reviewer: input.reviewer,
            same_role_as_implementer: input.same_role_as_implementer,
            blocking_count: input.blocking_count,
            warning_count: input.warning_count,
            file_line_evidence: input.file_line_evidence,
            all_blockings_resolved: input.all_blockings_resolved,
            artifact_path: input.artifact_path,
            created_at: now_string(),
        });
        ledger.push_history(
            "review",
            format!("review recorded from @{role} on {engine}/{model}"),
        );
    })
}

/// Record verification evidence on an existing ledger.
pub fn record_verification(project_root: &Path, input: VerificationInput) -> Result<GateLedger> {
    let thread_id = input.thread_id.clone();
    ensure_evidence_writes_allowed(project_root, &thread_id, "verification evidence")?;
    update_ledger(project_root, &thread_id, |ledger| {
        let command = input.command.trim().to_owned();
        ledger.verifications.push(GateVerification {
            command: command.clone(),
            ok: input.ok,
            evidence: input.evidence,
            created_at: now_string(),
        });
        ledger.push_history("verification", format!("verification recorded: {command}"));
    })
}

/// Record an authority-scoped plan review decision for the current plan SHA.
pub fn record_role_review(
    project_root: &Path,
    input: RoleReviewInput,
) -> Result<GateRoleReviewRecord> {
    let RoleReviewInput {
        thread_id,
        role,
        decision,
        reason,
    } = input;
    let thread_id = thread_id.trim();
    if thread_id.is_empty() {
        bail!("thread id cannot be empty");
    }
    ensure_evidence_writes_allowed(project_root, thread_id, "plan review evidence")?;
    let role = normalize_gate_role(&role)?;
    let reason = reason
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    if decision.blocks_signoff() && reason.is_none() {
        bail!("{} reviews require --reason", decision.label());
    }

    let coreroom_dir = project_root.join(COREROOM_DIR);
    ensure_gate_dirs(&coreroom_dir)?;
    let mut ledger = load_ledger(&coreroom_dir, thread_id)?;
    let plan = current_plan_artifact(project_root, &ledger)?;
    let config = load_authority_config(project_root)?;
    let entry = config
        .roles
        .get(&role)
        .with_context(|| format!("@{role} is not declared in .coreroom/config.toml"))?;
    let scopes = intersect_scopes(&entry.authority, &plan.scopes);
    if scopes.is_empty() {
        bail!(
            "@{role} has no authority over current plan scopes: {}",
            format_scopes(&plan.scopes)
        );
    }
    let role_config = config
        .role_config(&role, &coreroom_dir)
        .with_context(|| format!("@{role} has no resolved role config"))?;
    let model = role_config
        .model
        .filter(|model| !model.trim().is_empty())
        .with_context(|| {
            format!(
                "@{role} has no resolved model; set default_model or roles.{role}.model in config"
            )
        })?;
    let record = GateRoleReviewRecord {
        decision,
        reason,
        reviewer: GateActor {
            role: role.clone(),
            engine: role_config.engine,
            model,
            turn_id: None,
            thread_id: Some(ledger.thread_id.clone()),
        },
        plan_sha: plan.sha.clone(),
        plan_path: plan.path.clone(),
        scopes,
        created_at: now_string(),
    };

    write_role_review_record(project_root, &ledger.thread_id, &role, &record)?;
    ledger
        .role_reviews
        .retain(|review| review.reviewer.role != role);
    ledger.role_reviews.push(record.clone());
    if decision.blocks_signoff() {
        ledger.result = GateResult::Incomplete;
    }
    ledger.push_history(
        "plan_reviewed",
        format!(
            "@{role} recorded {} for {} ({})",
            decision.label(),
            record.plan_path,
            short_sha(&record.plan_sha)
        ),
    );
    save_ledger(&coreroom_dir, &ledger)?;
    write_active_thread(&coreroom_dir, &ledger.thread_id)?;
    Ok(record)
}

/// Record an explicit user override for a blocking authority-scoped review.
pub fn record_plan_override(
    project_root: &Path,
    input: PlanOverrideInput,
) -> Result<GatePlanOverride> {
    let PlanOverrideInput {
        thread_id,
        role,
        reason,
    } = input;
    let thread_id = thread_id.trim();
    if thread_id.is_empty() {
        bail!("thread id cannot be empty");
    }
    ensure_evidence_writes_allowed(project_root, thread_id, "plan review overrides")?;
    let role = normalize_gate_role(&role)?;
    let reason = reason.trim();
    if reason.is_empty() {
        bail!("override reason cannot be empty");
    }

    let coreroom_dir = project_root.join(COREROOM_DIR);
    ensure_gate_dirs(&coreroom_dir)?;
    let mut ledger = load_ledger(&coreroom_dir, thread_id)?;
    let status = plan_review_status(project_root, &ledger)?;
    let Some(reviewer) = status
        .required
        .iter()
        .find(|reviewer| reviewer.role == role)
    else {
        bail!(
            "@{role} is not required for current plan scopes: {}",
            format_scopes(&status.scopes)
        );
    };
    match (reviewer.decision, reviewer.stale) {
        (Some(decision), false) if decision.blocks_signoff() => {}
        (Some(_), true) => bail!("@{role} review is stale; re-review the current plan SHA instead"),
        (Some(PlanReviewDecision::Approve), false) => {
            bail!("@{role} has approved the current plan; nothing to override")
        }
        (None, _) => bail!("@{role} has not rejected the current plan; nothing to override"),
        _ => bail!("@{role} has no blocking review to override"),
    }

    let override_record = GatePlanOverride {
        role: role.clone(),
        reason: reason.to_owned(),
        plan_sha: status.plan_sha.clone(),
        created_at: now_string(),
    };
    write_plan_override_record(project_root, &ledger.thread_id, &role, &override_record)?;
    ledger
        .plan_overrides
        .retain(|entry| !(entry.role == role && entry.plan_sha == override_record.plan_sha));
    ledger.plan_overrides.push(override_record.clone());
    ledger.push_history(
        "plan_overridden",
        format!(
            "@{role} veto overridden for {}: {reason}",
            short_sha(&override_record.plan_sha)
        ),
    );
    save_ledger(&coreroom_dir, &ledger)?;
    write_active_thread(&coreroom_dir, &ledger.thread_id)?;
    Ok(override_record)
}

/// Explicitly advance or roll back a gate phase.
pub fn advance_phase(project_root: &Path, input: &PhaseAdvanceInput) -> Result<PhaseTransition> {
    let thread_id = input.thread_id.trim();
    if thread_id.is_empty() {
        bail!("thread id cannot be empty");
    }
    let actor = input.actor.trim();
    if actor.is_empty() {
        bail!("phase actor cannot be empty");
    }
    let coreroom_dir = project_root.join(COREROOM_DIR);
    ensure_gate_dirs(&coreroom_dir)?;
    let mut ledger = load_ledger(&coreroom_dir, thread_id)?;
    let from = ledger.phase;
    let to = input.to;
    if from == to {
        bail!("gate `{thread_id}` is already in phase `{}`", to.label());
    }
    if matches!(from, GatePhase::Closed | GatePhase::Rejected) {
        bail!(
            "gate `{thread_id}` is terminal in phase `{}`; create a new gate for new work",
            from.label()
        );
    }

    let rollback_reason = input
        .rollback_reason
        .as_deref()
        .map(str::trim)
        .filter(|reason| !reason.is_empty());
    if let Some(reason) = rollback_reason {
        if !from.can_rollback_to(to) {
            bail!(
                "cannot roll back gate `{thread_id}` from `{}` to `{}`; rollback target must be an earlier linear phase",
                from.label(),
                to.label()
            );
        }
        ledger.phase = to;
        ledger.push_history(
            "phase_rolled_back",
            format!("{} -> {} by {actor}: {reason}", from.label(), to.label()),
        );
    } else {
        if !from.can_advance_to(to) {
            bail!(
                "cannot advance gate `{thread_id}` from `{}` to `{}`; phases are linear and cannot be skipped",
                from.label(),
                to.label()
            );
        }
        validate_phase_transition(project_root, &ledger, from, to)?;
        ledger.phase = to;
        ledger.push_history(
            "phase_advanced",
            format!("{} -> {} by {actor}", from.label(), to.label()),
        );
    }

    let transition = PhaseTransition {
        thread_id: ledger.thread_id.clone(),
        from,
        to,
        actor: actor.to_owned(),
        rollback_reason: rollback_reason.map(ToOwned::to_owned),
    };
    ensure_phase_artifact(
        project_root,
        &ledger,
        from,
        to,
        actor,
        transition.rollback_reason.as_deref(),
    )?;
    save_ledger(&coreroom_dir, &ledger)?;
    write_active_thread(&coreroom_dir, &ledger.thread_id)?;
    Ok(transition)
}

/// Record a role-declared phase block on an existing gate.
pub fn record_phase_block(
    project_root: &Path,
    thread_id: &str,
    role: &str,
    reason: &str,
) -> Result<GatePhaseBlock> {
    let reason = reason.trim();
    if reason.is_empty() {
        bail!("phase block reason cannot be empty");
    }
    let role = role.trim().trim_start_matches('@');
    if role.is_empty() {
        bail!("phase block role cannot be empty");
    }
    let coreroom_dir = project_root.join(COREROOM_DIR);
    ensure_gate_dirs(&coreroom_dir)?;
    let mut ledger = load_ledger(&coreroom_dir, thread_id)?;
    let block = GatePhaseBlock {
        phase: ledger.phase,
        role: role.to_owned(),
        reason: reason.to_owned(),
        created_at: now_string(),
    };
    ledger.phase_blocks.push(block.clone());
    ledger.result = GateResult::Incomplete;
    ledger.push_history(
        "phase_blocked",
        format!("{} blocked by @{role}: {reason}", ledger.phase.label()),
    );
    save_ledger(&coreroom_dir, &ledger)?;
    write_active_thread(&coreroom_dir, &ledger.thread_id)?;
    Ok(block)
}

/// Record an explicit bypass entry.
pub fn record_bypass(
    project_root: &Path,
    thread_id: &str,
    gate: &str,
    reason: &str,
) -> Result<GateLedger> {
    if reason.trim().is_empty() {
        bail!("bypass reason cannot be empty");
    }
    let validation = validate(project_root, Some(thread_id)).ok();
    let blocked_reasons = validation.map(|v| v.reasons).unwrap_or_default();
    update_ledger(project_root, thread_id, |ledger| {
        ledger.bypasses.push(GateBypass {
            gate: gate.trim().to_owned(),
            reason: reason.trim().to_owned(),
            blocked_reasons,
            created_at: now_string(),
        });
        ledger.result = GateResult::Bypassed;
        ledger.push_history("bypass", format!("{gate} bypassed: {}", reason.trim()));
    })
}

/// Validate a gate ledger structurally.
pub fn validate(project_root: &Path, thread_id: Option<&str>) -> Result<GateValidation> {
    let ledger = load(project_root, thread_id)?;
    Ok(validate_ledger(project_root, &ledger))
}

/// Return the authority-scoped review status for the current plan artifact.
pub fn plan_review_status(project_root: &Path, ledger: &GateLedger) -> Result<PlanReviewStatus> {
    let plan = current_plan_artifact(project_root, ledger)?;
    let config = load_authority_config(project_root)?;
    let review_records = latest_role_review_records(project_root, ledger)?;
    let override_records = latest_plan_override_records(project_root, ledger)?;
    let mut required = Vec::new();
    let mut covered = BTreeSet::new();
    let mut roles: Vec<_> = config.roles.iter().collect();
    roles.sort_by_key(|(role, _)| *role);

    for (role, entry) in roles {
        let scopes = intersect_scopes(&entry.authority, &plan.scopes);
        if scopes.is_empty() {
            continue;
        }
        covered.extend(scopes.iter().copied());
        let review = review_records.get(role);
        let override_record = override_records.get(role);
        required.push(PlanReviewerStatus {
            role: role.clone(),
            scopes,
            decision: review.map(|record| record.decision),
            reason: review.and_then(|record| record.reason.clone()),
            stale: review.is_some_and(|record| record.plan_sha != plan.sha),
            overridden: override_record.is_some_and(|record| record.plan_sha == plan.sha),
        });
    }

    let uncovered_scopes = if required.is_empty() {
        Vec::new()
    } else {
        plan.scopes
            .iter()
            .copied()
            .filter(|scope| !covered.contains(scope))
            .collect()
    };
    let mut warnings = Vec::new();
    if required.is_empty() {
        warnings.push(format!(
            "plan scopes have no matching authority role: {}",
            format_scopes(&plan.scopes)
        ));
    }

    Ok(PlanReviewStatus {
        plan_path: plan.path,
        plan_sha: plan.sha,
        scopes: plan.scopes,
        required,
        uncovered_scopes,
        warnings,
    })
}

/// Close a gate only when validation passes, or when a bypass reason is provided.
pub fn close(
    project_root: &Path,
    thread_id: &str,
    bypass_reason: Option<&str>,
) -> Result<GateLedger> {
    let validation = validate(project_root, Some(thread_id))?;
    if validation.passed() {
        return update_ledger(project_root, thread_id, |ledger| {
            ledger.phase = GatePhase::Closed;
            ledger.result = GateResult::Pass;
            ledger.push_history("closed", "gate closed after validation pass");
        });
    }

    let Some(reason) = bypass_reason
        .map(str::trim)
        .filter(|reason| !reason.is_empty())
    else {
        bail!("{}", format_blocking_message(&validation));
    };

    update_ledger(project_root, thread_id, |ledger| {
        ledger.phase = GatePhase::Closed;
        ledger.result = GateResult::Bypassed;
        ledger.bypasses.push(GateBypass {
            gate: "completion".to_owned(),
            reason: reason.to_owned(),
            blocked_reasons: validation.reasons.clone(),
            created_at: now_string(),
        });
        ledger.push_history("closed_bypassed", format!("completion bypassed: {reason}"));
    })
}

/// Format a concise blocking message for completion guards.
#[must_use]
pub fn format_blocking_message(validation: &GateValidation) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{} gate incomplete:", validation.tier.label());
    for reason in &validation.reasons {
        let _ = writeln!(out, "- {reason}");
    }
    out.push_str("\nAsk host to resolve the gate, or bypass with an explicit reason.");
    out
}

/// Format a short status summary for CLI and host output.
#[must_use]
pub fn format_status(ledger: &GateLedger, validation: &GateValidation) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "thread: {}", ledger.thread_id);
    let _ = writeln!(out, "feature: {}", ledger.feature);
    let _ = writeln!(out, "tier: {}", ledger.tier.label());
    let _ = writeln!(out, "phase: {}", ledger.phase.label());
    let _ = writeln!(out, "result: {}", validation.result.label());
    let _ = writeln!(out, "artifacts: {}", ledger.artifacts.len());
    let _ = writeln!(out, "reviewers: {}", ledger.reviewers.len());
    let _ = writeln!(out, "verifications: {}", ledger.verifications.len());
    if let Some(status) = &validation.plan_review_status {
        let _ = writeln!(
            out,
            "\nplan review: {} ({})",
            status.plan_path,
            short_sha(&status.plan_sha)
        );
        let _ = writeln!(out, "scopes: {}", format_scopes(&status.scopes));
        if status.required.is_empty() {
            let _ = writeln!(out, "required reviewers: none");
        } else {
            let _ = writeln!(out, "required reviewers:");
            for reviewer in &status.required {
                let mut state = if reviewer.overridden {
                    "overridden".to_owned()
                } else if reviewer.stale {
                    "stale".to_owned()
                } else {
                    reviewer.decision.map_or_else(
                        || "unreviewed".to_owned(),
                        |decision| decision.label().to_owned(),
                    )
                };
                if let Some(reason) = &reviewer.reason {
                    state.push_str(": ");
                    state.push_str(reason);
                }
                let _ = writeln!(
                    out,
                    "- @{} [{}]: {}",
                    reviewer.role,
                    format_scopes(&reviewer.scopes),
                    state
                );
            }
        }
        if !status.uncovered_scopes.is_empty() {
            let _ = writeln!(
                out,
                "uncovered scopes: {}",
                format_scopes(&status.uncovered_scopes)
            );
        }
    }
    if let Some(block) = ledger.phase_blocks.last() {
        let _ = writeln!(
            out,
            "latest phase block: {} by @{} — {}",
            block.phase.label(),
            block.role,
            block.reason
        );
    }
    if !validation.reasons.is_empty() {
        let _ = writeln!(out, "\nblocking:");
        for reason in &validation.reasons {
            let _ = writeln!(out, "- {reason}");
        }
    }
    if !validation.warnings.is_empty() {
        let _ = writeln!(out, "\nwarnings:");
        for warning in &validation.warnings {
            let _ = writeln!(out, "- {warning}");
        }
    }
    out.trim_end().to_owned()
}

/// Runtime context appended to role prompts so host-led gates can use the current ids.
#[must_use]
pub fn runtime_prompt_context(
    role: &str,
    host_role: &str,
    turn_id: &str,
    thread_id: &str,
) -> String {
    let template_hint = ".coreroom/gate-templates/";
    let mut out = String::new();
    let _ = writeln!(out, "\n\n---\n\nCoreRoom runtime context:");
    let _ = writeln!(out, "- turn_id: {turn_id}");
    let _ = writeln!(out, "- thread_id: {thread_id}");
    if role == host_role {
        let _ = writeln!(
            out,
            "- For code-changing work, classify Tier 0/Tier 1 and drive SDLC gates conversationally."
        );
        let _ = writeln!(
            out,
            "- Tier 0/read-only work reports inline; do not write `.coreroom/` gate or review evidence unless the user explicitly asks for a ledger."
        );
        let _ = writeln!(
            out,
            "- For Tier 1, use `cr gate init --thread {thread_id} --tier 1 --feature \"...\"` and templates in `{template_hint}`."
        );
        let _ = writeln!(
            out,
            "- Before saying work is complete or ready, run `cr gate close --thread {thread_id}`; if blocked, report the gate reasons."
        );
    } else {
        let _ = writeln!(
            out,
            "- If reviewing Tier 1 work, record review evidence with `cr gate reviewer --thread {thread_id} ...`."
        );
        let _ = writeln!(
            out,
            "- For Tier 0/read-only review, cite evidence inline and do not write `.coreroom/` review artifacts."
        );
    }
    out
}

impl GateLedger {
    fn push_history(&mut self, event: impl Into<String>, detail: impl Into<String>) {
        self.updated_at = now_string();
        self.history.push(GateHistoryEntry {
            at: self.updated_at.clone(),
            event: event.into(),
            detail: detail.into(),
        });
    }
}

fn validate_ledger(project_root: &Path, ledger: &GateLedger) -> GateValidation {
    let mut reasons = Vec::new();
    let mut warnings = Vec::new();

    if ledger.schema_version != LEDGER_SCHEMA_VERSION {
        warnings.push(format!(
            "ledger schema_version={} differs from supported {}",
            ledger.schema_version, LEDGER_SCHEMA_VERSION
        ));
    }

    validate_artifact_paths(project_root, ledger, &mut reasons, &mut warnings);

    if ledger.tier == GateTier::Tier1 {
        validate_tier1(project_root, ledger, &mut reasons, &mut warnings);
    }

    let plan_review_status = match maybe_plan_review_status(project_root, ledger) {
        Ok(status) => {
            if let Some(status) = &status {
                warnings.extend(status.warnings.clone());
                if ledger.tier == GateTier::Tier1 {
                    reasons.extend(status.blocking_reasons());
                }
            }
            status
        }
        Err(error) => {
            reasons.push(format!("plan review status is invalid: {error:#}"));
            None
        }
    };

    let result = if reasons.is_empty() {
        if ledger.result == GateResult::Bypassed {
            GateResult::Bypassed
        } else {
            GateResult::Pass
        }
    } else if reasons.iter().any(|reason| reason.contains("missing")) {
        GateResult::Incomplete
    } else {
        GateResult::Fail
    };

    GateValidation {
        thread_id: ledger.thread_id.clone(),
        tier: ledger.tier,
        result,
        reasons,
        warnings,
        plan_review_status,
    }
}

fn validate_tier1(
    project_root: &Path,
    ledger: &GateLedger,
    reasons: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    if ledger.implementer.is_none() {
        reasons.push("implementer role/engine/model metadata is missing".to_owned());
    }

    for kind in [
        GateArtifactKind::Discovery,
        GateArtifactKind::Plan,
        GateArtifactKind::Review,
        GateArtifactKind::Signoff,
    ] {
        if !ledger
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == kind)
        {
            reasons.push(format!("{} artifact is missing", kind.label()));
        }
    }

    validate_plan_artifacts(project_root, ledger, reasons);
    validate_review_artifacts(project_root, ledger, reasons, warnings);
    validate_signoff_artifacts(project_root, ledger, reasons);
    validate_verifications(ledger, reasons);
    validate_cross_model_review(ledger, reasons);
}

fn validate_artifact_paths(
    project_root: &Path,
    ledger: &GateLedger,
    reasons: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    let line_ref_re = Regex::new(r"(?m)(?:^|[\s`(])([A-Za-z0-9_./-]+\.[A-Za-z0-9_./-]+):(\d+)")
        .expect("line-ref regex compiles");
    for artifact in &ledger.artifacts {
        let path = resolve_project_path(project_root, &artifact.path);
        if !path.is_file() {
            reasons.push(format!(
                "{} artifact path does not exist: {}",
                artifact.kind.label(),
                artifact.path
            ));
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            warnings.push(format!("could not read artifact {}", artifact.path));
            continue;
        };
        for cap in line_ref_re.captures_iter(&content) {
            let Some(file_match) = cap.get(1) else {
                continue;
            };
            let Some(line_match) = cap.get(2) else {
                continue;
            };
            let line_no = line_match.as_str().parse::<usize>().unwrap_or(0);
            let cited_path = resolve_project_path(project_root, file_match.as_str());
            if !cited_path.is_file() {
                reasons.push(format!(
                    "{} cites missing file {}:{}",
                    artifact.path,
                    file_match.as_str(),
                    line_no
                ));
                continue;
            }
            if line_no == 0 {
                reasons.push(format!(
                    "{} cites invalid line {}:{}",
                    artifact.path,
                    file_match.as_str(),
                    line_no
                ));
                continue;
            }
            let line_count =
                std::fs::read_to_string(&cited_path).map_or(0, |text| text.lines().count());
            if line_count > 0 && line_no > line_count {
                warnings.push(format!(
                    "{} cites stale line {}:{} (file has {} lines)",
                    artifact.path,
                    file_match.as_str(),
                    line_no,
                    line_count
                ));
            }
        }
    }
}

fn validate_plan_artifacts(project_root: &Path, ledger: &GateLedger, reasons: &mut Vec<String>) {
    for artifact in ledger
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == GateArtifactKind::Plan)
    {
        let path = resolve_project_path(project_root, &artifact.path);
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        if !content.to_ascii_lowercase().contains("sign-off checklist")
            && !content.to_ascii_lowercase().contains("signoff checklist")
        {
            reasons.push(format!(
                "plan artifact {} lacks a Sign-off Checklist section",
                artifact.path
            ));
        }
        let ids = extract_signoff_ids(&content);
        if ids.is_empty() {
            reasons.push(format!(
                "plan artifact {} has no SO-N sign-off rows",
                artifact.path
            ));
        }
        for line in content.lines().filter(|line| line.contains("SO-")) {
            if line.contains('|') {
                let columns = line
                    .split('|')
                    .map(str::trim)
                    .filter(|part| !part.is_empty())
                    .count();
                if columns < 4 {
                    reasons.push(format!(
                        "plan artifact {} has malformed sign-off row `{}`",
                        artifact.path,
                        line.trim()
                    ));
                }
            }
        }
    }
}

fn validate_review_artifacts(
    project_root: &Path,
    ledger: &GateLedger,
    reasons: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    let file_line_re = Regex::new(r"[A-Za-z0-9_./-]+\.[A-Za-z0-9_./-]+:\d+")
        .expect("review evidence regex compiles");
    for artifact in ledger
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == GateArtifactKind::Review)
    {
        let path = resolve_project_path(project_root, &artifact.path);
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let lower = content.to_ascii_lowercase();
        for field in [
            "reviewer_role",
            "engine",
            "model",
            "blocking_count",
            "warning_count",
            "cross_model_satisfied",
            "all_blockings_resolved",
        ] {
            if !lower.contains(field) {
                reasons.push(format!(
                    "review artifact {} missing `{field}`",
                    artifact.path
                ));
            }
        }
        if lower.contains("all_blockings_resolved: false")
            || lower.contains("\"all_blockings_resolved\": false")
        {
            reasons.push(format!(
                "review artifact {} has all_blockings_resolved=false",
                artifact.path
            ));
        }
        if lower.contains("cross_model_satisfied: false")
            || lower.contains("\"cross_model_satisfied\": false")
        {
            reasons.push(format!(
                "review artifact {} has cross_model_satisfied=false",
                artifact.path
            ));
        }
        if !file_line_re.is_match(&content) {
            warnings.push(format!(
                "review artifact {} has no visible file:line evidence",
                artifact.path
            ));
        }
    }

    for review in &ledger.reviewers {
        if review.blocking_count > 0 && !review.all_blockings_resolved {
            reasons.push(format!(
                "review by @{} has unresolved blocking findings",
                review.reviewer.role
            ));
        }
        if !review.file_line_evidence {
            reasons.push(format!(
                "review by @{} does not record file:line evidence",
                review.reviewer.role
            ));
        }
    }
}

fn validate_signoff_artifacts(project_root: &Path, ledger: &GateLedger, reasons: &mut Vec<String>) {
    let mut plan_ids = BTreeSet::new();
    for artifact in ledger
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == GateArtifactKind::Plan)
    {
        let path = resolve_project_path(project_root, &artifact.path);
        if let Ok(content) = std::fs::read_to_string(path) {
            plan_ids.extend(extract_signoff_ids(&content));
        }
    }
    if plan_ids.is_empty() {
        return;
    }
    let mut signoff_text = String::new();
    for artifact in ledger
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == GateArtifactKind::Signoff)
    {
        let path = resolve_project_path(project_root, &artifact.path);
        if let Ok(content) = std::fs::read_to_string(path) {
            signoff_text.push_str(&content);
            signoff_text.push('\n');
        }
    }
    for id in plan_ids {
        if !signoff_text.contains(&id) {
            reasons.push(format!("sign-off evidence missing {id} from plan"));
        }
    }
}

fn validate_verifications(ledger: &GateLedger, reasons: &mut Vec<String>) {
    if ledger.verifications.iter().all(|v| !v.ok) {
        reasons.push("no passing verification entry is recorded".to_owned());
    }
    for verification in &ledger.verifications {
        if verification.ok && !meaningful_evidence(&verification.evidence) {
            reasons.push(format!(
                "verification `{}` lacks actual output or cited evidence",
                verification.command
            ));
        }
    }
}

fn validate_cross_model_review(ledger: &GateLedger, reasons: &mut Vec<String>) {
    if ledger.reviewers.len() < 2 {
        reasons.push("Tier 1 cross-model review requires at least two reviewer turns".to_owned());
    }
    let Some(implementer) = &ledger.implementer else {
        return;
    };
    let Some(implementer_family) = model_family(implementer.engine, &implementer.model) else {
        reasons.push(format!(
            "implementer model family is unsupported or missing: {}",
            implementer.model
        ));
        return;
    };
    let mut has_independent = false;
    let mut has_different_family = false;
    let mut unsupported = Vec::new();
    for review in &ledger.reviewers {
        let independent =
            review.reviewer.role != implementer.role && !review.same_role_as_implementer;
        if independent {
            has_independent = true;
        }
        match model_family(review.reviewer.engine, &review.reviewer.model) {
            Some(family) if independent && family != implementer_family => {
                has_different_family = true;
            }
            Some(_) => {}
            None => unsupported.push(format!(
                "@{} model `{}`",
                review.reviewer.role, review.reviewer.model
            )),
        }
    }
    if !has_independent {
        reasons.push(
            "same role reviewing its own output does not satisfy independent review".to_owned(),
        );
    }
    if !has_different_family {
        reasons.push(
            "Tier 1 cross-model review requires at least one independent reviewer from a different model family"
                .to_owned(),
        );
    }
    for item in unsupported {
        reasons.push(format!(
            "reviewer {item} has unsupported or missing model family"
        ));
    }
}

fn model_family(_engine: Engine, model: &str) -> Option<String> {
    let normalized = model.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }
    if normalized.contains("claude")
        || normalized.contains("opus")
        || normalized.contains("sonnet")
        || normalized.contains("haiku")
    {
        return Some("anthropic-claude".to_owned());
    }
    if normalized.contains("gpt")
        || normalized.contains("o3")
        || normalized.contains("o4")
        || normalized.contains("codex")
    {
        return Some("openai".to_owned());
    }
    if normalized.contains("gemini") {
        return Some("google-gemini".to_owned());
    }
    None
}

fn extract_signoff_ids(content: &str) -> BTreeSet<String> {
    Regex::new(r"\bSO-\d+\b")
        .expect("signoff regex compiles")
        .find_iter(content)
        .map(|m| m.as_str().to_owned())
        .collect()
}

fn meaningful_evidence(evidence: &str) -> bool {
    let trimmed = evidence.trim();
    if trimmed.len() < 12 {
        return false;
    }
    let normalized = trimmed.to_ascii_lowercase();
    !matches!(
        normalized.as_str(),
        "ok" | "verified" | "pass" | "passed" | "done" | "green"
    )
}

fn validate_phase_transition(
    project_root: &Path,
    ledger: &GateLedger,
    from: GatePhase,
    to: GatePhase,
) -> Result<()> {
    match (from, to) {
        (GatePhase::Plan, GatePhase::Review) => {
            let path = phase_artifact_path(project_root, &ledger.thread_id, GatePhase::Plan);
            if !path.is_file() {
                bail!(
                    "cannot advance gate `{}` from `plan` to `review`; plan artifact is missing at {}",
                    ledger.thread_id,
                    path.display()
                );
            }
            let label = project_relative_path(project_root, &path);
            plan_artifact_from_path(&label, &path)?;
            Ok(())
        }
        (GatePhase::Review, GatePhase::Signoff) => {
            let status = plan_review_status(project_root, ledger)?;
            let blockers = status.blocking_reasons();
            if blockers.is_empty() {
                return Ok(());
            }
            let mut message = format!(
                "cannot advance gate `{}` from `review` to `signoff`; plan reviews are incomplete:",
                ledger.thread_id
            );
            for blocker in blockers {
                let _ = write!(message, "\n- {blocker}");
            }
            bail!("{message}");
        }
        _ => Ok(()),
    }
}

fn maybe_plan_review_status(
    project_root: &Path,
    ledger: &GateLedger,
) -> Result<Option<PlanReviewStatus>> {
    if current_plan_path(project_root, ledger).is_none() {
        return Ok(None);
    }
    plan_review_status(project_root, ledger).map(Some)
}

fn current_plan_artifact(project_root: &Path, ledger: &GateLedger) -> Result<PlanArtifactInfo> {
    let Some((path, absolute_path)) = current_plan_path(project_root, ledger) else {
        bail!(
            "plan artifact is missing; expected {}",
            phase_artifact_path(project_root, &ledger.thread_id, GatePhase::Plan).display()
        );
    };
    plan_artifact_from_path(&path, &absolute_path)
}

fn current_plan_path(project_root: &Path, ledger: &GateLedger) -> Option<(String, PathBuf)> {
    let phase_path = phase_artifact_path(project_root, &ledger.thread_id, GatePhase::Plan);
    if phase_path.is_file() {
        return Some((project_relative_path(project_root, &phase_path), phase_path));
    }
    ledger
        .artifacts
        .iter()
        .rev()
        .filter(|artifact| artifact.kind == GateArtifactKind::Plan)
        .map(|artifact| {
            (
                artifact.path.clone(),
                resolve_project_path(project_root, &artifact.path),
            )
        })
        .find(|(_, path)| path.is_file())
}

fn plan_artifact_from_path(path: &str, absolute_path: &Path) -> Result<PlanArtifactInfo> {
    let content = std::fs::read_to_string(absolute_path)
        .with_context(|| format!("reading plan artifact {}", absolute_path.display()))?;
    let scopes = parse_plan_scopes(&content)
        .with_context(|| format!("parsing scopes from plan artifact {path}"))?;
    Ok(PlanArtifactInfo {
        path: path.to_owned(),
        sha: sha256_file(absolute_path)?,
        scopes,
    })
}

fn parse_plan_scopes(content: &str) -> Result<Vec<AuthorityScope>> {
    let Some(frontmatter) = plan_frontmatter(content) else {
        bail!("plan artifact must start with frontmatter declaring `scopes`");
    };
    let lines: Vec<&str> = frontmatter.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = strip_inline_comment(line).trim();
        if let Some(rest) = trimmed.strip_prefix("scopes:") {
            let rest = rest.trim();
            if !rest.is_empty() {
                return parse_scope_values(rest);
            }
            let mut values = Vec::new();
            for child in lines.iter().skip(index + 1) {
                let child_trimmed = strip_inline_comment(child).trim();
                if child_trimmed.is_empty() {
                    continue;
                }
                if let Some(value) = child_trimmed.strip_prefix('-') {
                    values.push(value.trim().to_owned());
                    continue;
                }
                break;
            }
            return parse_scope_tokens(values);
        }
        if let Some(rest) = trimmed.strip_prefix("scopes =") {
            return parse_scope_values(rest.trim());
        }
    }
    bail!("plan artifact frontmatter must declare `scopes`");
}

fn plan_frontmatter(content: &str) -> Option<String> {
    let mut lines = content.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let mut frontmatter = String::new();
    for line in lines {
        if matches!(line.trim(), "---" | "...") {
            return Some(frontmatter);
        }
        frontmatter.push_str(line);
        frontmatter.push('\n');
    }
    None
}

fn parse_scope_values(raw: &str) -> Result<Vec<AuthorityScope>> {
    let raw = raw.trim();
    if raw.is_empty() {
        bail!("`scopes` cannot be empty");
    }
    let values = if raw.starts_with('[') && raw.ends_with(']') {
        raw.trim_start_matches('[')
            .trim_end_matches(']')
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    } else if raw.contains(',') {
        raw.split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    } else {
        vec![raw.to_owned()]
    };
    parse_scope_tokens(values)
}

fn parse_scope_tokens(values: Vec<String>) -> Result<Vec<AuthorityScope>> {
    if values.is_empty() {
        bail!("`scopes` cannot be empty");
    }
    let mut parsed = BTreeSet::new();
    for value in values {
        let normalized = value
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .trim()
            .to_ascii_lowercase()
            .replace('_', "-");
        let Some(scope) = AuthorityScope::parse(&normalized) else {
            bail!(
                "unknown plan scope `{}`; expected one of: {}",
                value.trim(),
                AuthorityScope::expected_values()
            );
        };
        parsed.insert(scope);
    }
    Ok(AuthorityScope::ALL
        .into_iter()
        .filter(|scope| parsed.contains(scope))
        .collect())
}

fn strip_inline_comment(line: &str) -> &str {
    line.split_once('#').map_or(line, |(before, _)| before)
}

fn load_authority_config(project_root: &Path) -> Result<Config> {
    Config::load(project_root).with_context(|| "loading role authority config")
}

fn latest_role_review_records(
    project_root: &Path,
    ledger: &GateLedger,
) -> Result<BTreeMap<String, GateRoleReviewRecord>> {
    let mut records = BTreeMap::new();
    for record in &ledger.role_reviews {
        insert_latest_role_review(&mut records, record.clone());
    }
    let dir = role_reviews_dir(project_root, &ledger.thread_id);
    if dir.is_dir() {
        for entry in
            std::fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("toml") {
                continue;
            }
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            let record: GateRoleReviewRecord =
                toml::from_str(&content).with_context(|| format!("parsing {}", path.display()))?;
            insert_latest_role_review(&mut records, record);
        }
    }
    Ok(records)
}

fn insert_latest_role_review(
    records: &mut BTreeMap<String, GateRoleReviewRecord>,
    record: GateRoleReviewRecord,
) {
    let role = record.reviewer.role.clone();
    let replace = records
        .get(&role)
        .is_none_or(|existing| record.created_at >= existing.created_at);
    if replace {
        records.insert(role, record);
    }
}

fn latest_plan_override_records(
    project_root: &Path,
    ledger: &GateLedger,
) -> Result<BTreeMap<String, GatePlanOverride>> {
    let mut records = BTreeMap::new();
    for record in &ledger.plan_overrides {
        insert_latest_plan_override(&mut records, record.clone());
    }
    let dir = plan_overrides_dir(project_root, &ledger.thread_id);
    if dir.is_dir() {
        for entry in
            std::fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("toml") {
                continue;
            }
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            let record: GatePlanOverride =
                toml::from_str(&content).with_context(|| format!("parsing {}", path.display()))?;
            insert_latest_plan_override(&mut records, record);
        }
    }
    Ok(records)
}

fn insert_latest_plan_override(
    records: &mut BTreeMap<String, GatePlanOverride>,
    record: GatePlanOverride,
) {
    let replace = records
        .get(&record.role)
        .is_none_or(|existing| record.created_at >= existing.created_at);
    if replace {
        records.insert(record.role.clone(), record);
    }
}

fn write_role_review_record(
    project_root: &Path,
    thread_id: &str,
    role: &str,
    record: &GateRoleReviewRecord,
) -> Result<()> {
    let path = role_review_path(project_root, thread_id, role);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let content = toml::to_string_pretty(record)?;
    std::fs::write(&path, ensure_trailing_newline(&content))
        .with_context(|| format!("writing {}", path.display()))
}

fn write_plan_override_record(
    project_root: &Path,
    thread_id: &str,
    role: &str,
    record: &GatePlanOverride,
) -> Result<()> {
    let path = plan_override_path(project_root, thread_id, role);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let content = toml::to_string_pretty(record)?;
    std::fs::write(&path, ensure_trailing_newline(&content))
        .with_context(|| format!("writing {}", path.display()))
}

fn role_review_path(project_root: &Path, thread_id: &str, role: &str) -> PathBuf {
    role_reviews_dir(project_root, thread_id).join(format!("{}.toml", sanitize_thread_id(role)))
}

fn role_reviews_dir(project_root: &Path, thread_id: &str) -> PathBuf {
    project_root
        .join(COREROOM_DIR)
        .join(GATES_DIR)
        .join(sanitize_thread_id(thread_id))
        .join("reviews")
}

fn plan_override_path(project_root: &Path, thread_id: &str, role: &str) -> PathBuf {
    plan_overrides_dir(project_root, thread_id).join(format!("{}.toml", sanitize_thread_id(role)))
}

fn plan_overrides_dir(project_root: &Path, thread_id: &str) -> PathBuf {
    project_root
        .join(COREROOM_DIR)
        .join(GATES_DIR)
        .join(sanitize_thread_id(thread_id))
        .join("overrides")
}

fn intersect_scopes(left: &[AuthorityScope], right: &[AuthorityScope]) -> Vec<AuthorityScope> {
    let left: BTreeSet<_> = left.iter().copied().collect();
    let right: BTreeSet<_> = right.iter().copied().collect();
    AuthorityScope::ALL
        .into_iter()
        .filter(|scope| left.contains(scope) && right.contains(scope))
        .collect()
}

fn format_scopes(scopes: &[AuthorityScope]) -> String {
    let set: BTreeSet<_> = scopes.iter().copied().collect();
    let labels: Vec<_> = AuthorityScope::ALL
        .iter()
        .copied()
        .filter(|scope| set.contains(scope))
        .map(AuthorityScope::as_str)
        .collect();
    if labels.is_empty() {
        "(none)".to_owned()
    } else {
        labels.join(", ")
    }
}

fn normalize_gate_role(role: &str) -> Result<String> {
    let role = role.trim().trim_start_matches('@');
    if role.is_empty() {
        bail!("role cannot be empty");
    }
    Ok(role.to_owned())
}

fn project_relative_path(project_root: &Path, path: &Path) -> String {
    path.strip_prefix(project_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn short_sha(sha: &str) -> &str {
    sha.get(..12).unwrap_or(sha)
}

fn update_ledger(
    project_root: &Path,
    thread_id: &str,
    update: impl FnOnce(&mut GateLedger),
) -> Result<GateLedger> {
    let coreroom_dir = project_root.join(COREROOM_DIR);
    ensure_gate_dirs(&coreroom_dir)?;
    let mut ledger = load_ledger(&coreroom_dir, thread_id)?;
    update(&mut ledger);
    save_ledger(&coreroom_dir, &ledger)?;
    write_active_thread(&coreroom_dir, &ledger.thread_id)?;
    Ok(ledger)
}

fn ensure_gate_dirs(coreroom_dir: &Path) -> Result<()> {
    let gates_dir = coreroom_dir.join(GATES_DIR);
    std::fs::create_dir_all(&gates_dir)
        .with_context(|| format!("creating {}", gates_dir.display()))?;
    Ok(())
}

fn load_ledger(coreroom_dir: &Path, thread_id: &str) -> Result<GateLedger> {
    let path = ledger_path(coreroom_dir, thread_id);
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("reading gate ledger {}", path.display()))?;
    serde_json::from_str(&content).with_context(|| format!("parsing {}", path.display()))
}

fn save_ledger(coreroom_dir: &Path, ledger: &GateLedger) -> Result<()> {
    ensure_gate_dirs(coreroom_dir)?;
    let path = ledger_path(coreroom_dir, &ledger.thread_id);
    let content = serde_json::to_string_pretty(ledger)?;
    std::fs::write(&path, ensure_trailing_newline(&content))
        .with_context(|| format!("writing {}", path.display()))
}

fn selected_thread_id(coreroom_dir: &Path, explicit: Option<&str>) -> Result<String> {
    if let Some(thread_id) = explicit.map(str::trim).filter(|id| !id.is_empty()) {
        return Ok(thread_id.to_owned());
    }
    let active_path = coreroom_dir.join(GATES_DIR).join(ACTIVE_GATE_FILE);
    let content = std::fs::read_to_string(&active_path)
        .with_context(|| format!("reading active gate pointer {}", active_path.display()))?;
    let thread_id = content.trim();
    if thread_id.is_empty() {
        bail!("active gate pointer is empty; pass --thread explicitly");
    }
    Ok(thread_id.to_owned())
}

fn write_active_thread(coreroom_dir: &Path, thread_id: &str) -> Result<()> {
    ensure_gate_dirs(coreroom_dir)?;
    let active_path = coreroom_dir.join(GATES_DIR).join(ACTIVE_GATE_FILE);
    std::fs::write(&active_path, format!("{thread_id}\n"))
        .with_context(|| format!("writing {}", active_path.display()))
}

/// Return `.coreroom/gates/<thread>/<phase>.md` for a phase artifact.
#[must_use]
pub fn phase_artifact_path(project_root: &Path, thread_id: &str, phase: GatePhase) -> PathBuf {
    project_root
        .join(COREROOM_DIR)
        .join(GATES_DIR)
        .join(sanitize_thread_id(thread_id))
        .join(format!("{}.md", phase.label()))
}

fn ensure_phase_artifact(
    project_root: &Path,
    ledger: &GateLedger,
    from: GatePhase,
    to: GatePhase,
    actor: &str,
    rollback_reason: Option<&str>,
) -> Result<PathBuf> {
    let path = phase_artifact_path(project_root, &ledger.thread_id, to);
    if path.exists() {
        return Ok(path);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let mut body = String::new();
    let _ = writeln!(body, "# {} phase", to.label());
    let _ = writeln!(body);
    let _ = writeln!(body, "thread: {}", ledger.thread_id);
    let _ = writeln!(body, "feature: {}", ledger.feature);
    let _ = writeln!(body, "from: {}", from.label());
    let _ = writeln!(body, "to: {}", to.label());
    let _ = writeln!(body, "actor: {actor}");
    let _ = writeln!(body, "created_at: {}", now_string());
    if let Some(reason) = rollback_reason {
        let _ = writeln!(body, "rollback_reason: {reason}");
    }
    let _ = writeln!(body);
    let _ = writeln!(body, "## Notes");
    let _ = writeln!(body);
    let _ = writeln!(
        body,
        "- Fill in the evidence, decisions, or blockers for this phase."
    );
    std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

fn ledger_path(coreroom_dir: &Path, thread_id: &str) -> PathBuf {
    coreroom_dir
        .join(GATES_DIR)
        .join(format!("{}.json", sanitize_thread_id(thread_id)))
}

fn sanitize_thread_id(thread_id: &str) -> String {
    thread_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn resolve_project_path(project_root: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_root.join(path)
    }
}

fn ensure_trailing_newline(content: &str) -> String {
    if content.ends_with('\n') {
        content.to_owned()
    } else {
        format!("{content}\n")
    }
}

fn now_string() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Return counts by artifact kind for display and tests.
#[must_use]
pub fn artifact_counts(ledger: &GateLedger) -> HashMap<GateArtifactKind, usize> {
    let mut counts = HashMap::new();
    for artifact in &ledger.artifacts {
        *counts.entry(artifact.kind).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_project_file(root: &Path, path: &str, content: &str) {
        let path = root.join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    fn actor(role: &str, engine: Engine, model: &str) -> GateActor {
        GateActor {
            role: role.to_owned(),
            engine,
            model: model.to_owned(),
            turn_id: Some(format!("turn-{role}")),
            thread_id: Some("thread-1".to_owned()),
        }
    }

    fn write_gate_config(root: &Path, body: &str, roles: &[&str]) {
        let coreroom = root.join(COREROOM_DIR);
        std::fs::create_dir_all(coreroom.join("roles")).unwrap();
        std::fs::write(coreroom.join("config.toml"), body).unwrap();
        for role in roles {
            std::fs::write(
                coreroom.join("roles").join(format!("{role}.md")),
                "priors\n",
            )
            .unwrap();
        }
    }

    fn write_plan(root: &Path, thread_id: &str, body: &str) {
        let path = phase_artifact_path(root, thread_id, GatePhase::Plan);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, body).unwrap();
    }

    fn plan_with_scopes(scopes: &str, body: &str) -> String {
        format!(
            "---\nscopes: [{scopes}]\n---\n\n# Plan\n\n{body}\n\n## Sign-off Checklist\n\n| ID | Owner | Check | Evidence |\n| - | - | - | - |\n| SO-1 | host | Ready | TBD |\n"
        )
    }

    #[test]
    fn parses_plan_scope_frontmatter() {
        let scopes = parse_plan_scopes(
            "---\nscopes:\n  - infra\n  - data_policy\n  - \"deployment\"\n---\nbody\n",
        )
        .unwrap();

        assert_eq!(
            scopes,
            vec![
                AuthorityScope::Deployment,
                AuthorityScope::Infra,
                AuthorityScope::DataPolicy
            ]
        );
    }

    #[test]
    fn plan_review_status_warns_when_no_authority_role_matches() {
        let tmp = tempfile::tempdir().unwrap();
        write_gate_config(
            tmp.path(),
            r#"
default_engine = "cc"
default_model = "claude-sonnet-4"
host_role = "pm"

[roles.pm]

[roles.backend]
authority = ["dependencies"]
"#,
            &["pm", "backend"],
        );
        init(
            tmp.path(),
            GateInit {
                thread_id: "thread-1".to_owned(),
                feature: "plan signoff".to_owned(),
                tier: GateTier::Tier1,
                phase: GatePhase::Plan,
                implementer: Some(actor("pm", Engine::Cc, "claude-sonnet-4")),
            },
        )
        .unwrap();
        write_plan(tmp.path(), "thread-1", &plan_with_scopes("infra", "change"));
        let ledger = load(tmp.path(), Some("thread-1")).unwrap();

        let status = plan_review_status(tmp.path(), &ledger).unwrap();

        assert!(status.required.is_empty());
        assert!(status.blocking_reasons().is_empty());
        assert!(status
            .warnings
            .iter()
            .any(|warning| warning.contains("no matching authority role")));
    }

    #[test]
    fn partial_scope_coverage_blocks_signoff() {
        let tmp = tempfile::tempdir().unwrap();
        write_gate_config(
            tmp.path(),
            r#"
default_engine = "cc"
default_model = "claude-sonnet-4"
host_role = "pm"

[roles.pm]

[roles.sre]
authority = ["infra"]
"#,
            &["pm", "sre"],
        );
        init(
            tmp.path(),
            GateInit {
                thread_id: "thread-1".to_owned(),
                feature: "plan signoff".to_owned(),
                tier: GateTier::Tier1,
                phase: GatePhase::Review,
                implementer: Some(actor("pm", Engine::Cc, "claude-sonnet-4")),
            },
        )
        .unwrap();
        write_plan(
            tmp.path(),
            "thread-1",
            &plan_with_scopes("infra, deployment", "change"),
        );
        let ledger = load(tmp.path(), Some("thread-1")).unwrap();

        let status = plan_review_status(tmp.path(), &ledger).unwrap();

        assert_eq!(status.uncovered_scopes, vec![AuthorityScope::Deployment]);
        assert!(status
            .blocking_reasons()
            .iter()
            .any(|reason| reason.contains("lack authority coverage")));
    }

    #[test]
    fn plan_sha_change_invalidates_role_review() {
        let tmp = tempfile::tempdir().unwrap();
        write_gate_config(
            tmp.path(),
            r#"
default_engine = "cc"
default_model = "claude-sonnet-4"
host_role = "pm"

[roles.pm]

[roles.sre]
engine = "codex"
model = "gpt-5"
authority = ["infra"]
"#,
            &["pm", "sre"],
        );
        init(
            tmp.path(),
            GateInit {
                thread_id: "thread-1".to_owned(),
                feature: "plan signoff".to_owned(),
                tier: GateTier::Tier1,
                phase: GatePhase::Review,
                implementer: Some(actor("pm", Engine::Cc, "claude-sonnet-4")),
            },
        )
        .unwrap();
        write_plan(tmp.path(), "thread-1", &plan_with_scopes("infra", "first"));
        record_role_review(
            tmp.path(),
            RoleReviewInput {
                thread_id: "thread-1".to_owned(),
                role: "sre".to_owned(),
                decision: PlanReviewDecision::Approve,
                reason: None,
            },
        )
        .unwrap();
        write_plan(
            tmp.path(),
            "thread-1",
            &plan_with_scopes("infra", "changed"),
        );
        let ledger = load(tmp.path(), Some("thread-1")).unwrap();

        let status = plan_review_status(tmp.path(), &ledger).unwrap();

        assert!(status.required[0].stale);
        assert!(status
            .blocking_reasons()
            .iter()
            .any(|reason| reason.contains("stale")));
    }

    #[test]
    fn tier1_missing_evidence_is_incomplete() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(COREROOM_DIR)).unwrap();
        init(
            tmp.path(),
            GateInit {
                thread_id: "thread-1".to_owned(),
                feature: "change login flow".to_owned(),
                tier: GateTier::Tier1,
                phase: GatePhase::Discovery,
                implementer: Some(actor("host", Engine::Cc, "claude-sonnet-4")),
            },
        )
        .unwrap();

        let validation = validate(tmp.path(), Some("thread-1")).unwrap();

        assert_eq!(validation.result, GateResult::Incomplete);
        assert!(validation
            .reasons
            .iter()
            .any(|reason| reason.contains("discovery artifact is missing")));
        assert!(validation
            .reasons
            .iter()
            .any(|reason| reason.contains("at least two reviewer turns")));
    }

    #[test]
    fn phase_transition_advances_linearly_and_creates_artifacts() {
        let tmp = tempfile::tempdir().unwrap();
        init(
            tmp.path(),
            GateInit {
                thread_id: "thread-1".to_owned(),
                feature: "phase flow".to_owned(),
                tier: GateTier::Tier1,
                phase: GatePhase::Intake,
                implementer: None,
            },
        )
        .unwrap();

        assert!(phase_artifact_path(tmp.path(), "thread-1", GatePhase::Intake).is_file());
        let transition = advance_phase(
            tmp.path(),
            &PhaseAdvanceInput {
                thread_id: "thread-1".to_owned(),
                to: GatePhase::Discovery,
                actor: "user".to_owned(),
                rollback_reason: None,
            },
        )
        .unwrap();

        assert_eq!(transition.from, GatePhase::Intake);
        assert_eq!(transition.to, GatePhase::Discovery);
        assert_eq!(
            load(tmp.path(), Some("thread-1")).unwrap().phase,
            GatePhase::Discovery
        );
        let artifact_path = phase_artifact_path(tmp.path(), "thread-1", GatePhase::Discovery);
        let artifact = std::fs::read_to_string(artifact_path).unwrap();
        assert!(artifact.contains("# discovery phase"));
        assert!(artifact.contains("from: intake"));
        assert!(artifact.contains("to: discovery"));
        assert!(artifact.contains("actor: user"));
    }

    #[test]
    fn phase_transition_rejects_skips_and_regressions_without_rollback() {
        let tmp = tempfile::tempdir().unwrap();
        init(
            tmp.path(),
            GateInit {
                thread_id: "thread-1".to_owned(),
                feature: "phase flow".to_owned(),
                tier: GateTier::Tier1,
                phase: GatePhase::Intake,
                implementer: None,
            },
        )
        .unwrap();

        let skip_error = advance_phase(
            tmp.path(),
            &PhaseAdvanceInput {
                thread_id: "thread-1".to_owned(),
                to: GatePhase::Plan,
                actor: "user".to_owned(),
                rollback_reason: None,
            },
        )
        .unwrap_err();
        assert!(skip_error.to_string().contains("cannot advance"));
        assert_eq!(
            load(tmp.path(), Some("thread-1")).unwrap().phase,
            GatePhase::Intake
        );

        advance_phase(
            tmp.path(),
            &PhaseAdvanceInput {
                thread_id: "thread-1".to_owned(),
                to: GatePhase::Discovery,
                actor: "user".to_owned(),
                rollback_reason: None,
            },
        )
        .unwrap();
        advance_phase(
            tmp.path(),
            &PhaseAdvanceInput {
                thread_id: "thread-1".to_owned(),
                to: GatePhase::Plan,
                actor: "user".to_owned(),
                rollback_reason: None,
            },
        )
        .unwrap();
        let regression_error = advance_phase(
            tmp.path(),
            &PhaseAdvanceInput {
                thread_id: "thread-1".to_owned(),
                to: GatePhase::Discovery,
                actor: "user".to_owned(),
                rollback_reason: None,
            },
        )
        .unwrap_err();
        assert!(regression_error.to_string().contains("cannot advance"));
        assert_eq!(
            load(tmp.path(), Some("thread-1")).unwrap().phase,
            GatePhase::Plan
        );
    }

    #[test]
    fn phase_transition_allows_rollback_with_reason() {
        let tmp = tempfile::tempdir().unwrap();
        init(
            tmp.path(),
            GateInit {
                thread_id: "thread-1".to_owned(),
                feature: "phase flow".to_owned(),
                tier: GateTier::Tier1,
                phase: GatePhase::Intake,
                implementer: None,
            },
        )
        .unwrap();
        for phase in [GatePhase::Discovery, GatePhase::Plan] {
            advance_phase(
                tmp.path(),
                &PhaseAdvanceInput {
                    thread_id: "thread-1".to_owned(),
                    to: phase,
                    actor: "user".to_owned(),
                    rollback_reason: None,
                },
            )
            .unwrap();
        }

        let transition = advance_phase(
            tmp.path(),
            &PhaseAdvanceInput {
                thread_id: "thread-1".to_owned(),
                to: GatePhase::Discovery,
                actor: "user".to_owned(),
                rollback_reason: Some("plan needs another pass".to_owned()),
            },
        )
        .unwrap();

        assert_eq!(transition.from, GatePhase::Plan);
        assert_eq!(transition.to, GatePhase::Discovery);
        assert_eq!(
            transition.rollback_reason.as_deref(),
            Some("plan needs another pass")
        );
        let ledger = load(tmp.path(), Some("thread-1")).unwrap();
        assert_eq!(ledger.phase, GatePhase::Discovery);
        assert!(ledger
            .history
            .iter()
            .any(|entry| entry.event == "phase_rolled_back"));
    }

    #[test]
    fn rejected_is_only_a_review_or_signoff_branch() {
        let tmp = tempfile::tempdir().unwrap();
        init(
            tmp.path(),
            GateInit {
                thread_id: "thread-1".to_owned(),
                feature: "phase flow".to_owned(),
                tier: GateTier::Tier1,
                phase: GatePhase::Review,
                implementer: None,
            },
        )
        .unwrap();

        advance_phase(
            tmp.path(),
            &PhaseAdvanceInput {
                thread_id: "thread-1".to_owned(),
                to: GatePhase::Rejected,
                actor: "user".to_owned(),
                rollback_reason: None,
            },
        )
        .unwrap();
        assert_eq!(
            load(tmp.path(), Some("thread-1")).unwrap().phase,
            GatePhase::Rejected
        );

        init(
            tmp.path(),
            GateInit {
                thread_id: "thread-2".to_owned(),
                feature: "phase flow".to_owned(),
                tier: GateTier::Tier1,
                phase: GatePhase::Implement,
                implementer: None,
            },
        )
        .unwrap();
        let error = advance_phase(
            tmp.path(),
            &PhaseAdvanceInput {
                thread_id: "thread-2".to_owned(),
                to: GatePhase::Rejected,
                actor: "user".to_owned(),
                rollback_reason: None,
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("cannot advance"));
    }

    #[test]
    fn record_phase_block_appends_block_and_status() {
        let tmp = tempfile::tempdir().unwrap();
        init(
            tmp.path(),
            GateInit {
                thread_id: "thread-1".to_owned(),
                feature: "phase flow".to_owned(),
                tier: GateTier::Tier1,
                phase: GatePhase::Plan,
                implementer: None,
            },
        )
        .unwrap();

        let block =
            record_phase_block(tmp.path(), "thread-1", "@security", "scope mismatch").unwrap();

        assert_eq!(block.phase, GatePhase::Plan);
        assert_eq!(block.role, "security");
        assert_eq!(block.reason, "scope mismatch");
        let ledger = load(tmp.path(), Some("thread-1")).unwrap();
        assert_eq!(ledger.phase_blocks, vec![block]);
        assert_eq!(ledger.result, GateResult::Incomplete);
        let validation = validate(tmp.path(), Some("thread-1")).unwrap();
        assert!(
            format_status(&ledger, &validation).contains("latest phase block: plan by @security")
        );
    }

    #[test]
    fn tier0_rejects_hidden_review_evidence_writes() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(COREROOM_DIR)).unwrap();
        init(
            tmp.path(),
            GateInit {
                thread_id: "thread-1".to_owned(),
                feature: "read-only project review".to_owned(),
                tier: GateTier::Tier0,
                phase: GatePhase::Review,
                implementer: Some(actor("host", Engine::Cc, "claude-sonnet-4")),
            },
        )
        .unwrap();

        let error = record_review(
            tmp.path(),
            ReviewInput {
                thread_id: "thread-1".to_owned(),
                reviewer: actor("security", Engine::Codex, "gpt-5"),
                same_role_as_implementer: false,
                blocking_count: 0,
                warning_count: 1,
                file_line_evidence: true,
                all_blockings_resolved: true,
                artifact_path: Some("docs/review.md".to_owned()),
            },
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("Tier 0/read-only gates do not record review evidence"));
        let ledger = load(tmp.path(), Some("thread-1")).unwrap();
        assert!(ledger.reviewers.is_empty());
        assert!(ledger.artifacts.is_empty());
        assert_eq!(ledger.history.len(), 1);
    }

    #[test]
    fn tier0_rejects_artifact_and_verification_writes() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(COREROOM_DIR)).unwrap();
        init(
            tmp.path(),
            GateInit {
                thread_id: "thread-1".to_owned(),
                feature: "read-only project review".to_owned(),
                tier: GateTier::Tier0,
                phase: GatePhase::Review,
                implementer: Some(actor("host", Engine::Cc, "claude-sonnet-4")),
            },
        )
        .unwrap();

        let artifact_error = record_artifact(
            tmp.path(),
            ArtifactInput {
                thread_id: "thread-1".to_owned(),
                kind: GateArtifactKind::Review,
                path: "docs/review.md".to_owned(),
                role: Some("security".to_owned()),
                turn_id: Some("turn-security".to_owned()),
            },
        )
        .unwrap_err();
        let verification_error = record_verification(
            tmp.path(),
            VerificationInput {
                thread_id: "thread-1".to_owned(),
                command: "cargo test".to_owned(),
                ok: true,
                evidence: "not run; read-only review".to_owned(),
            },
        )
        .unwrap_err();

        assert!(artifact_error
            .to_string()
            .contains("Tier 0/read-only gates do not record gate artifacts"));
        assert!(verification_error
            .to_string()
            .contains("Tier 0/read-only gates do not record verification evidence"));
        let ledger = load(tmp.path(), Some("thread-1")).unwrap();
        assert!(ledger.artifacts.is_empty());
        assert!(ledger.verifications.is_empty());
        assert_eq!(ledger.history.len(), 1);
    }

    #[test]
    fn cross_model_review_can_satisfy_tier1() {
        let tmp = tempfile::tempdir().unwrap();
        write_gate_config(
            tmp.path(),
            r#"
default_engine = "cc"
default_model = "claude-sonnet-4"
host_role = "host"

[roles.host]
"#,
            &["host"],
        );
        write_project_file(tmp.path(), "src/lib.rs", "pub fn ok() {}\n");
        write_project_file(tmp.path(), "docs/research.md", "Evidence: src/lib.rs:1\n");
        write_project_file(
            tmp.path(),
            "docs/plan.md",
            &plan_with_scopes("infra", "builds"),
        );
        write_project_file(
            tmp.path(),
            "docs/review.md",
            "reviewer_role: security\nengine: codex\nmodel: gpt-5\nblocking_count: 0\nwarning_count: 0\ncross_model_satisfied: true\nall_blockings_resolved: true\nEvidence: src/lib.rs:1\n",
        );
        write_project_file(tmp.path(), "docs/signoff.md", "SO-1: cargo test passed\n");

        init(
            tmp.path(),
            GateInit {
                thread_id: "thread-1".to_owned(),
                feature: "change login flow".to_owned(),
                tier: GateTier::Tier1,
                phase: GatePhase::Review,
                implementer: Some(actor("host", Engine::Cc, "claude-sonnet-4")),
            },
        )
        .unwrap();
        for (kind, path) in [
            (GateArtifactKind::Discovery, "docs/research.md"),
            (GateArtifactKind::Plan, "docs/plan.md"),
            (GateArtifactKind::Review, "docs/review.md"),
            (GateArtifactKind::Signoff, "docs/signoff.md"),
        ] {
            record_artifact(
                tmp.path(),
                ArtifactInput {
                    thread_id: "thread-1".to_owned(),
                    kind,
                    path: path.to_owned(),
                    role: Some("host".to_owned()),
                    turn_id: Some("turn-host".to_owned()),
                },
            )
            .unwrap();
        }
        record_review(
            tmp.path(),
            ReviewInput {
                thread_id: "thread-1".to_owned(),
                reviewer: actor("security", Engine::Codex, "gpt-5"),
                same_role_as_implementer: false,
                blocking_count: 0,
                warning_count: 0,
                file_line_evidence: true,
                all_blockings_resolved: true,
                artifact_path: Some("docs/review.md".to_owned()),
            },
        )
        .unwrap();
        record_review(
            tmp.path(),
            ReviewInput {
                thread_id: "thread-1".to_owned(),
                reviewer: actor("qa", Engine::Cc, "claude-opus-4"),
                same_role_as_implementer: false,
                blocking_count: 0,
                warning_count: 0,
                file_line_evidence: true,
                all_blockings_resolved: true,
                artifact_path: None,
            },
        )
        .unwrap();
        record_verification(
            tmp.path(),
            VerificationInput {
                thread_id: "thread-1".to_owned(),
                command: "cargo test --all-features --locked".to_owned(),
                ok: true,
                evidence: "test result: ok. 12 passed; 0 failed".to_owned(),
            },
        )
        .unwrap();

        let validation = validate(tmp.path(), Some("thread-1")).unwrap();

        assert_eq!(validation.result, GateResult::Pass, "{validation:#?}");
    }

    #[test]
    fn close_requires_bypass_reason_when_incomplete() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(COREROOM_DIR)).unwrap();
        init(
            tmp.path(),
            GateInit {
                thread_id: "thread-1".to_owned(),
                feature: "change login flow".to_owned(),
                tier: GateTier::Tier1,
                phase: GatePhase::Discovery,
                implementer: None,
            },
        )
        .unwrap();

        let error = close(tmp.path(), "thread-1", None).unwrap_err();
        assert!(error.to_string().contains("Tier 1 gate incomplete"));

        let ledger = close(tmp.path(), "thread-1", Some("user accepted missing review")).unwrap();
        assert_eq!(ledger.result, GateResult::Bypassed);
        assert_eq!(ledger.phase, GatePhase::Closed);
        assert_eq!(ledger.bypasses.len(), 1);
    }
}
