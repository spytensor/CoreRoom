//! SDLC gate ledgers, artifact validation, and completion guards.
//!
//! The gate layer is intentionally structural. It can prove that required
//! evidence is present, named, and linked to local files; it does not claim
//! an implementation is semantically correct.

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::adapter::Engine;
use crate::config::CODEROOM_DIR;

/// Subdirectory inside `.coderoom/` that stores per-thread gate ledgers.
pub const GATES_DIR: &str = "gates";

/// Subdirectory inside `.coderoom/` that stores reusable SDLC gate templates.
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
    /// File name written under `.coderoom/gate-templates/`.
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
    /// Initial code-first research.
    Research,
    /// Plan creation and user review.
    Plan,
    /// Implementation is in progress.
    Implementation,
    /// Peer or cross-model review is in progress.
    Review,
    /// Pre-commit verification is in progress.
    Precommit,
    /// Sign-off evidence is being collected.
    Signoff,
    /// Gate is closed.
    Closed,
}

impl GatePhase {
    /// Parse a user-facing phase token.
    pub fn parse(input: &str) -> Result<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "research" => Ok(Self::Research),
            "plan" => Ok(Self::Plan),
            "implementation" | "impl" => Ok(Self::Implementation),
            "review" => Ok(Self::Review),
            "precommit" | "pre-commit" => Ok(Self::Precommit),
            "signoff" | "sign-off" => Ok(Self::Signoff),
            "closed" | "close" => Ok(Self::Closed),
            other => bail!("unknown phase `{other}`"),
        }
    }

    /// Compact label used in CLI output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Research => "research",
            Self::Plan => "plan",
            Self::Implementation => "implementation",
            Self::Review => "review",
            Self::Precommit => "precommit",
            Self::Signoff => "signoff",
            Self::Closed => "closed",
        }
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
    /// Research artifact.
    Research,
    /// Implementation plan artifact.
    Plan,
    /// Plan review artifact.
    PlanReview,
    /// Code review artifact.
    Review,
    /// Pre-commit verification artifact.
    Precommit,
    /// Sign-off artifact.
    Signoff,
}

impl GateArtifactKind {
    /// Parse a user-facing artifact kind.
    pub fn parse(input: &str) -> Result<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "research" => Ok(Self::Research),
            "plan" => Ok(Self::Plan),
            "plan-review" | "plan_review" | "planreview" => Ok(Self::PlanReview),
            "review" | "code-review" | "code_review" => Ok(Self::Review),
            "precommit" | "pre-commit" => Ok(Self::Precommit),
            "signoff" | "sign-off" => Ok(Self::Signoff),
            other => bail!("unknown artifact kind `{other}`"),
        }
    }

    /// Compact label used in CLI output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Research => "research",
            Self::Plan => "plan",
            Self::PlanReview => "plan-review",
            Self::Review => "review",
            Self::Precommit => "precommit",
            Self::Signoff => "signoff",
        }
    }
}

/// Actor metadata for implementers and reviewers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GateActor {
    /// CodeRoom role name.
    pub role: String,
    /// Engine used by that role.
    pub engine: Engine,
    /// Engine model identifier.
    pub model: String,
    /// CodeRoom turn id, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    /// CodeRoom thread id, when known.
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
    /// CodeRoom turn id, when known.
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

/// Persistent SDLC gate ledger stored under `.coderoom/gates/`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GateLedger {
    /// Schema version.
    pub schema_version: u32,
    /// CodeRoom thread id or user-provided work id.
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
    /// Recorded verification evidence.
    #[serde(default)]
    pub verifications: Vec<GateVerification>,
    /// Explicit accepted risks and bypasses.
    #[serde(default)]
    pub bypasses: Vec<GateBypass>,
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
}

impl GateValidation {
    /// Whether the validation passed.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.result == GateResult::Pass
    }
}

/// Outcome of installing gate templates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemplateInstallOutcome {
    /// Files written.
    pub written: usize,
    /// Existing files skipped.
    pub skipped: usize,
}

/// Install default SDLC gate templates under `.coderoom/gate-templates/`.
pub fn install_templates(coderoom_dir: &Path, overwrite: bool) -> Result<TemplateInstallOutcome> {
    let dir = coderoom_dir.join(GATE_TEMPLATES_DIR);
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
    let coderoom_dir = project_root.join(CODEROOM_DIR);
    ensure_gate_dirs(&coderoom_dir)?;
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
        verifications: Vec::new(),
        bypasses: Vec::new(),
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
    save_ledger(&coderoom_dir, &ledger)?;
    write_active_thread(&coderoom_dir, &ledger.thread_id)?;
    Ok(ledger)
}

/// Load a selected ledger, defaulting to the active gate.
pub fn load(project_root: &Path, thread_id: Option<&str>) -> Result<GateLedger> {
    let coderoom_dir = project_root.join(CODEROOM_DIR);
    let thread_id = selected_thread_id(&coderoom_dir, thread_id)?;
    load_ledger(&coderoom_dir, &thread_id)
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
    let template_hint = ".coderoom/gate-templates/";
    let mut out = String::new();
    let _ = writeln!(out, "\n\n---\n\nCodeRoom runtime context:");
    let _ = writeln!(out, "- turn_id: {turn_id}");
    let _ = writeln!(out, "- thread_id: {thread_id}");
    if role == host_role {
        let _ = writeln!(
            out,
            "- For code-changing work, classify Tier 0/Tier 1 and drive SDLC gates conversationally."
        );
        let _ = writeln!(
            out,
            "- Tier 0/read-only work reports inline; do not write `.coderoom/` gate or review evidence unless the user explicitly asks for a ledger."
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
            "- For Tier 0/read-only review, cite evidence inline and do not write `.coderoom/` review artifacts."
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
        GateArtifactKind::Research,
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

fn update_ledger(
    project_root: &Path,
    thread_id: &str,
    update: impl FnOnce(&mut GateLedger),
) -> Result<GateLedger> {
    let coderoom_dir = project_root.join(CODEROOM_DIR);
    ensure_gate_dirs(&coderoom_dir)?;
    let mut ledger = load_ledger(&coderoom_dir, thread_id)?;
    update(&mut ledger);
    save_ledger(&coderoom_dir, &ledger)?;
    write_active_thread(&coderoom_dir, &ledger.thread_id)?;
    Ok(ledger)
}

fn ensure_gate_dirs(coderoom_dir: &Path) -> Result<()> {
    let gates_dir = coderoom_dir.join(GATES_DIR);
    std::fs::create_dir_all(&gates_dir)
        .with_context(|| format!("creating {}", gates_dir.display()))?;
    Ok(())
}

fn load_ledger(coderoom_dir: &Path, thread_id: &str) -> Result<GateLedger> {
    let path = ledger_path(coderoom_dir, thread_id);
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("reading gate ledger {}", path.display()))?;
    serde_json::from_str(&content).with_context(|| format!("parsing {}", path.display()))
}

fn save_ledger(coderoom_dir: &Path, ledger: &GateLedger) -> Result<()> {
    ensure_gate_dirs(coderoom_dir)?;
    let path = ledger_path(coderoom_dir, &ledger.thread_id);
    let content = serde_json::to_string_pretty(ledger)?;
    std::fs::write(&path, ensure_trailing_newline(&content))
        .with_context(|| format!("writing {}", path.display()))
}

fn selected_thread_id(coderoom_dir: &Path, explicit: Option<&str>) -> Result<String> {
    if let Some(thread_id) = explicit.map(str::trim).filter(|id| !id.is_empty()) {
        return Ok(thread_id.to_owned());
    }
    let active_path = coderoom_dir.join(GATES_DIR).join(ACTIVE_GATE_FILE);
    let content = std::fs::read_to_string(&active_path)
        .with_context(|| format!("reading active gate pointer {}", active_path.display()))?;
    let thread_id = content.trim();
    if thread_id.is_empty() {
        bail!("active gate pointer is empty; pass --thread explicitly");
    }
    Ok(thread_id.to_owned())
}

fn write_active_thread(coderoom_dir: &Path, thread_id: &str) -> Result<()> {
    ensure_gate_dirs(coderoom_dir)?;
    let active_path = coderoom_dir.join(GATES_DIR).join(ACTIVE_GATE_FILE);
    std::fs::write(&active_path, format!("{thread_id}\n"))
        .with_context(|| format!("writing {}", active_path.display()))
}

fn ledger_path(coderoom_dir: &Path, thread_id: &str) -> PathBuf {
    coderoom_dir
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

    #[test]
    fn tier1_missing_evidence_is_incomplete() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(CODEROOM_DIR)).unwrap();
        init(
            tmp.path(),
            GateInit {
                thread_id: "thread-1".to_owned(),
                feature: "change login flow".to_owned(),
                tier: GateTier::Tier1,
                phase: GatePhase::Research,
                implementer: Some(actor("host", Engine::Cc, "claude-sonnet-4")),
            },
        )
        .unwrap();

        let validation = validate(tmp.path(), Some("thread-1")).unwrap();

        assert_eq!(validation.result, GateResult::Incomplete);
        assert!(validation
            .reasons
            .iter()
            .any(|reason| reason.contains("research artifact is missing")));
        assert!(validation
            .reasons
            .iter()
            .any(|reason| reason.contains("at least two reviewer turns")));
    }

    #[test]
    fn tier0_rejects_hidden_review_evidence_writes() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(CODEROOM_DIR)).unwrap();
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
        std::fs::create_dir_all(tmp.path().join(CODEROOM_DIR)).unwrap();
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
        std::fs::create_dir_all(tmp.path().join(CODEROOM_DIR)).unwrap();
        write_project_file(tmp.path(), "src/lib.rs", "pub fn ok() {}\n");
        write_project_file(tmp.path(), "docs/research.md", "Evidence: src/lib.rs:1\n");
        write_project_file(
            tmp.path(),
            "docs/plan.md",
            "## Sign-off Checklist\n\n| ID | Predicate | Method | Pass |\n| SO-1 | builds | cargo test | pass |\n",
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
            (GateArtifactKind::Research, "docs/research.md"),
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
        std::fs::create_dir_all(tmp.path().join(CODEROOM_DIR)).unwrap();
        init(
            tmp.path(),
            GateInit {
                thread_id: "thread-1".to_owned(),
                feature: "change login flow".to_owned(),
                tier: GateTier::Tier1,
                phase: GatePhase::Research,
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
