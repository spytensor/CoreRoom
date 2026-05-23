//! Host-managed action layer for project-level engineering control.
//!
//! This module gives `@host` a structured way to classify, propose, audit, and
//! gate project-level actions. It deliberately does not add a user-facing
//! command surface; commands remain automation/debug/recovery plumbing.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// Current host action schema version.
pub const HOST_ACTION_SCHEMA_VERSION: u32 = 1;

/// Project-level action category owned by `@host`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum HostActionKind {
    /// Classify user intent before any persistent state change.
    ClassifyUserIntent,
    /// Create a GitHub Issue from a WorkOrder draft.
    #[serde(rename = "create-github-issue")]
    CreateGitHubIssue,
    /// Bind a WorkOrder to an existing GitHub Issue.
    #[serde(rename = "bind-github-issue")]
    BindGitHubIssue,
    /// Bind WorkOrder state to issue, branch, PR, gate, or tracker facts.
    BindWorkOrder,
    /// Register a project source.
    RegisterSource,
    /// Refresh or re-pin a registered source.
    RefreshSource,
    /// Build a WorkOrder-scoped ContextPack.
    BuildContextPack,
    /// Collect an Evidence Packet from structural evidence.
    CollectEvidencePacket,
    /// Prepare a PR-ready evidence summary or completion claim.
    PreparePrEvidenceSummary,
    /// Update a milestone tracker checkbox or Evidence Ledger row.
    UpdateTracker,
    /// Ask the user for input when work is blocked.
    RequestHumanInput,
    /// Override an authority-scoped role veto.
    OverrideRoleVeto,
    /// Claim release readiness.
    ClaimReleaseReadiness,
    /// Change product constitution or locked architecture decisions.
    ChangeConstitution,
    /// Replace GitHub Issues/PRs/CI as the engineering fact source.
    ReplaceGithubFacts,
    /// Mutate project state silently without host/user confirmation boundaries.
    SilentProjectMutation,
}

impl HostActionKind {
    /// Stable persisted label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::ClassifyUserIntent => "classify-user-intent",
            Self::CreateGitHubIssue => "create-github-issue",
            Self::BindGitHubIssue => "bind-github-issue",
            Self::BindWorkOrder => "bind-work-order",
            Self::RegisterSource => "register-source",
            Self::RefreshSource => "refresh-source",
            Self::BuildContextPack => "build-context-pack",
            Self::CollectEvidencePacket => "collect-evidence-packet",
            Self::PreparePrEvidenceSummary => "prepare-pr-evidence-summary",
            Self::UpdateTracker => "update-tracker",
            Self::RequestHumanInput => "request-human-input",
            Self::OverrideRoleVeto => "override-role-veto",
            Self::ClaimReleaseReadiness => "claim-release-readiness",
            Self::ChangeConstitution => "change-constitution",
            Self::ReplaceGithubFacts => "replace-github-facts",
            Self::SilentProjectMutation => "silent-project-mutation",
        }
    }

    /// A-017 confirmation rule for this action.
    pub const fn confirmation_rule(self) -> ActionConfirmationRule {
        match self {
            Self::ClassifyUserIntent | Self::CollectEvidencePacket | Self::RequestHumanInput => {
                ActionConfirmationRule::NoConfirmation
            }
            Self::CreateGitHubIssue
            | Self::BindGitHubIssue
            | Self::BindWorkOrder
            | Self::RegisterSource
            | Self::RefreshSource
            | Self::BuildContextPack
            | Self::PreparePrEvidenceSummary
            | Self::UpdateTracker
            | Self::OverrideRoleVeto
            | Self::ClaimReleaseReadiness => ActionConfirmationRule::ConfirmationRequired,
            Self::ChangeConstitution => ActionConfirmationRule::HumanOnly,
            Self::ReplaceGithubFacts | Self::SilentProjectMutation => {
                ActionConfirmationRule::Forbidden
            }
        }
    }

    /// Whether the action mutates or claims project-level engineering state.
    pub const fn mutates_project_state(self) -> bool {
        !matches!(
            self,
            Self::ClassifyUserIntent | Self::RequestHumanInput | Self::CollectEvidencePacket
        )
    }

    /// Whether this action must be executed by `@host`.
    pub const fn host_only(self) -> bool {
        true
    }
}

/// Whether a request is only a proposal or asks to execute the action.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum ActionIntent {
    /// Role proposes an action for host review.
    Propose,
    /// Host attempts to execute or persist the action.
    Execute,
}

impl ActionIntent {
    /// Stable persisted label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Propose => "propose",
            Self::Execute => "execute",
        }
    }
}

/// Confirmation policy outcome from A-017.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum ActionConfirmationRule {
    /// Host may perform the action without asking first.
    NoConfirmation,
    /// Host must ask the user before executing.
    ConfirmationRequired,
    /// User must perform or explicitly own this decision outside host action execution.
    HumanOnly,
    /// The action is never permitted.
    Forbidden,
}

impl ActionConfirmationRule {
    /// Stable persisted label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::NoConfirmation => "no-confirmation",
            Self::ConfirmationRequired => "confirmation-required",
            Self::HumanOnly => "human-only",
            Self::Forbidden => "forbidden",
        }
    }
}

/// Runtime confirmation status for an action attempt.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum ActionConfirmationStatus {
    /// No confirmation is required.
    NotRequired,
    /// Confirmation is required and has not been supplied.
    RequiredPending,
    /// The user confirmed the action.
    Confirmed,
    /// This is reserved for the user, not an executable host action.
    HumanOnly,
    /// Action was denied.
    Denied,
}

impl ActionConfirmationStatus {
    /// Stable persisted label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::NotRequired => "not-required",
            Self::RequiredPending => "required-pending",
            Self::Confirmed => "confirmed",
            Self::HumanOnly => "human-only",
            Self::Denied => "denied",
        }
    }
}

/// Final action outcome.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum ActionOutcome {
    /// Action may execute.
    Allowed,
    /// Action is a proposal routed to host, not an execution.
    Proposed,
    /// Action is waiting on explicit user confirmation.
    ConfirmationRequired,
    /// Action must be performed or decided by the user.
    HumanOnly,
    /// Action is blocked on missing context or a stale session.
    Blocked,
    /// Action is not permitted.
    Forbidden,
    /// Action failed safety controls.
    Failed,
}

impl ActionOutcome {
    /// Stable persisted label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Proposed => "proposed",
            Self::ConfirmationRequired => "confirmation-required",
            Self::HumanOnly => "human-only",
            Self::Blocked => "blocked",
            Self::Forbidden => "forbidden",
            Self::Failed => "failed",
        }
    }
}

/// Safety finding produced by the action loop hardening layer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ActionSafetyFinding {
    /// Machine-readable finding code.
    pub code: String,
    /// Human-readable finding detail for host output.
    pub message: String,
}

impl ActionSafetyFinding {
    /// Create a safety finding.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

/// Safety limits for one action attempt.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ActionSafetyLimits {
    /// Maximum attempts before circuit breaker trips.
    pub max_attempts: u32,
    /// Session age in turns after which mutating actions block for refresh.
    pub stale_after_turns: u32,
    /// Repeated identical action count that emits a warning.
    pub stuck_loop_warning_after: u32,
    /// Repeated identical action count that trips the circuit breaker.
    pub circuit_breaker_repeated_actions: u32,
}

impl Default for ActionSafetyLimits {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            stale_after_turns: 12,
            stuck_loop_warning_after: 2,
            circuit_breaker_repeated_actions: 4,
        }
    }
}

/// Runtime attempt state used by safety controls.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ActionAttemptState {
    /// Number of times this action has been attempted.
    #[serde(default)]
    pub attempts: u32,
    /// Session age in host turns since state was last refreshed.
    #[serde(default)]
    pub session_age_turns: u32,
    /// Count of repeated identical action fingerprints.
    #[serde(default)]
    pub repeated_action_count: u32,
    /// Explicit blocker reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
}

/// Structured host action request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HostActionRequest {
    /// Schema version.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Stable action id.
    pub id: String,
    /// Role asking for the action.
    pub actor_role: String,
    /// Configured host role.
    pub host_role: String,
    /// Proposal or execution attempt.
    pub intent: ActionIntent,
    /// Action category.
    pub kind: HostActionKind,
    /// Target object, such as `#215`, `WO-0215`, or `source:docs`.
    pub target: String,
    /// Why this action exists.
    pub reason: String,
    /// Input facts used by the action.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inputs: BTreeMap<String, String>,
    /// User who confirmed the action, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed_by: Option<String>,
    /// Attempt state for safety evaluation.
    #[serde(default)]
    pub attempt: ActionAttemptState,
    /// Rollback hint that will be copied into the audit event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollback_hint: Option<String>,
}

impl HostActionRequest {
    /// Create a minimal request.
    pub fn new(
        id: impl Into<String>,
        actor_role: impl Into<String>,
        host_role: impl Into<String>,
        intent: ActionIntent,
        kind: HostActionKind,
        target: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: HOST_ACTION_SCHEMA_VERSION,
            id: id.into(),
            actor_role: actor_role.into(),
            host_role: host_role.into(),
            intent,
            kind,
            target: target.into(),
            reason: reason.into(),
            inputs: BTreeMap::new(),
            confirmed_by: None,
            attempt: ActionAttemptState::default(),
            rollback_hint: None,
        }
    }

    /// Validate structural request fields.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != HOST_ACTION_SCHEMA_VERSION {
            bail!(
                "unsupported HostAction schemaVersion {}; expected {}",
                self.schema_version,
                HOST_ACTION_SCHEMA_VERSION
            );
        }
        ensure_nonempty("id", &self.id)?;
        ensure_role("actorRole", &self.actor_role)?;
        ensure_role("hostRole", &self.host_role)?;
        ensure_nonempty("target", &self.target)?;
        ensure_nonempty("reason", &self.reason)?;
        if let Some(confirmed_by) = &self.confirmed_by {
            ensure_nonempty("confirmedBy", confirmed_by)?;
        }
        for (key, value) in &self.inputs {
            ensure_nonempty("inputs key", key)?;
            ensure_nonempty("inputs value", value)?;
        }
        Ok(())
    }

    /// Whether the actor is the configured host.
    pub fn is_host_actor(&self) -> bool {
        self.actor_role == self.host_role
    }
}

/// Decision for one host action request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HostActionDecision {
    /// A-017 confirmation rule.
    pub rule: ActionConfirmationRule,
    /// Final outcome.
    pub outcome: ActionOutcome,
    /// Whether execution may continue.
    pub can_execute: bool,
    /// Confirmation state.
    pub confirmation_status: ActionConfirmationStatus,
    /// Human-readable reason.
    pub reason: String,
    /// Safety findings.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub safety_findings: Vec<ActionSafetyFinding>,
}

/// Audit event generated for every action evaluation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ActionAuditEvent {
    /// Schema version.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Action id.
    pub action_id: String,
    /// Actor role.
    pub actor_role: String,
    /// Configured host role.
    pub host_role: String,
    /// Proposal or execution attempt.
    pub intent: ActionIntent,
    /// Action category.
    pub kind: HostActionKind,
    /// Action target.
    pub target: String,
    /// Input facts included with the request.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inputs: BTreeMap<String, String>,
    /// Confirmation rule used for the decision.
    pub rule: ActionConfirmationRule,
    /// Final outcome.
    pub outcome: ActionOutcome,
    /// Confirmation status.
    pub confirmation_status: ActionConfirmationStatus,
    /// User who confirmed the action, if present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed_by: Option<String>,
    /// Decision output.
    pub output: String,
    /// Rollback hint for persisted or external effects.
    pub rollback_hint: String,
    /// Safety findings recorded for audit.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub safety_findings: Vec<ActionSafetyFinding>,
}

/// Result of evaluating a host action request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HostActionResult {
    /// Original request.
    pub request: HostActionRequest,
    /// Decision.
    pub decision: HostActionDecision,
    /// Audit event.
    pub audit_event: ActionAuditEvent,
}

impl HostActionResult {
    /// Render a host-facing status line.
    pub fn render_host_summary(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "HostAction {} - {}",
            self.request.id,
            self.request.kind.label()
        );
        let _ = writeln!(out, "actor: @{}", self.request.actor_role);
        let _ = writeln!(out, "intent: {}", self.request.intent.label());
        let _ = writeln!(out, "target: {}", self.request.target);
        let _ = writeln!(out, "rule: {}", self.decision.rule.label());
        let _ = writeln!(out, "outcome: {}", self.decision.outcome.label());
        let _ = writeln!(
            out,
            "confirmation: {}",
            self.decision.confirmation_status.label()
        );
        let _ = writeln!(out, "canExecute: {}", self.decision.can_execute);
        let _ = writeln!(out, "reason: {}", self.decision.reason);
        if self.decision.safety_findings.is_empty() {
            let _ = writeln!(out, "safety: none");
        } else {
            let _ = writeln!(out, "safety:");
            for finding in &self.decision.safety_findings {
                let _ = writeln!(out, "- {}: {}", finding.code, finding.message);
            }
        }
        out
    }
}

/// Evaluate one host action request with default safety limits.
pub fn evaluate_host_action(request: HostActionRequest) -> Result<HostActionResult> {
    evaluate_host_action_with_limits(request, ActionSafetyLimits::default())
}

/// Evaluate one host action request.
pub fn evaluate_host_action_with_limits(
    request: HostActionRequest,
    limits: ActionSafetyLimits,
) -> Result<HostActionResult> {
    request.validate()?;
    let rule = request.kind.confirmation_rule();
    let safety_findings = safety_findings(&request, limits);
    let decision = decide_action(&request, rule, safety_findings.clone(), limits);
    let audit_event = ActionAuditEvent {
        schema_version: HOST_ACTION_SCHEMA_VERSION,
        action_id: request.id.clone(),
        actor_role: request.actor_role.clone(),
        host_role: request.host_role.clone(),
        intent: request.intent,
        kind: request.kind,
        target: request.target.clone(),
        inputs: request.inputs.clone(),
        rule: decision.rule,
        outcome: decision.outcome,
        confirmation_status: decision.confirmation_status,
        confirmed_by: request.confirmed_by.clone(),
        output: decision.reason.clone(),
        rollback_hint: request
            .rollback_hint
            .clone()
            .unwrap_or_else(|| default_rollback_hint(request.kind)),
        safety_findings,
    };

    Ok(HostActionResult {
        request,
        decision,
        audit_event,
    })
}

fn decide_action(
    request: &HostActionRequest,
    rule: ActionConfirmationRule,
    safety_findings: Vec<ActionSafetyFinding>,
    limits: ActionSafetyLimits,
) -> HostActionDecision {
    if safety_findings
        .iter()
        .any(|finding| finding.code == "circuit-breaker")
    {
        return decision(
            rule,
            ActionOutcome::Failed,
            false,
            ActionConfirmationStatus::Denied,
            "action failed safety controls: circuit breaker tripped",
            safety_findings,
        );
    }

    if request.attempt.blocker.is_some() {
        return decision(
            rule,
            ActionOutcome::Blocked,
            false,
            confirmation_status_for_rule(rule, request.confirmed_by.as_deref()),
            "action is blocked and must request human input",
            safety_findings,
        );
    }

    if request.kind.mutates_project_state()
        && request.intent == ActionIntent::Execute
        && request.attempt.session_age_turns > limits.stale_after_turns
    {
        return decision(
            rule,
            ActionOutcome::Blocked,
            false,
            confirmation_status_for_rule(rule, request.confirmed_by.as_deref()),
            "session state is stale; host must refresh facts before mutating project state",
            safety_findings,
        );
    }

    if request.intent == ActionIntent::Propose {
        return decision(
            rule,
            ActionOutcome::Proposed,
            false,
            confirmation_status_for_proposal(rule),
            "proposal recorded for host review; no project-level mutation executed",
            safety_findings,
        );
    }

    if request.kind.host_only() && !request.is_host_actor() {
        return decision(
            ActionConfirmationRule::Forbidden,
            ActionOutcome::Forbidden,
            false,
            ActionConfirmationStatus::Denied,
            "non-host roles may propose project-level actions but cannot execute them",
            safety_findings,
        );
    }

    match rule {
        ActionConfirmationRule::NoConfirmation => decision(
            rule,
            ActionOutcome::Allowed,
            true,
            ActionConfirmationStatus::NotRequired,
            "action allowed without confirmation under A-017",
            safety_findings,
        ),
        ActionConfirmationRule::ConfirmationRequired => {
            if request.confirmed_by.is_some() {
                decision(
                    rule,
                    ActionOutcome::Allowed,
                    true,
                    ActionConfirmationStatus::Confirmed,
                    "action allowed after explicit user confirmation",
                    safety_findings,
                )
            } else {
                decision(
                    rule,
                    ActionOutcome::ConfirmationRequired,
                    false,
                    ActionConfirmationStatus::RequiredPending,
                    "action requires explicit user confirmation before execution",
                    safety_findings,
                )
            }
        }
        ActionConfirmationRule::HumanOnly => decision(
            rule,
            ActionOutcome::HumanOnly,
            false,
            ActionConfirmationStatus::HumanOnly,
            "action is human-only; host may summarize but must not execute it",
            safety_findings,
        ),
        ActionConfirmationRule::Forbidden => decision(
            rule,
            ActionOutcome::Forbidden,
            false,
            ActionConfirmationStatus::Denied,
            "action is forbidden by host-led engineering control policy",
            safety_findings,
        ),
    }
}

fn decision(
    rule: ActionConfirmationRule,
    outcome: ActionOutcome,
    can_execute: bool,
    confirmation_status: ActionConfirmationStatus,
    reason: impl Into<String>,
    safety_findings: Vec<ActionSafetyFinding>,
) -> HostActionDecision {
    HostActionDecision {
        rule,
        outcome,
        can_execute,
        confirmation_status,
        reason: reason.into(),
        safety_findings,
    }
}

fn confirmation_status_for_rule(
    rule: ActionConfirmationRule,
    confirmed_by: Option<&str>,
) -> ActionConfirmationStatus {
    match rule {
        ActionConfirmationRule::NoConfirmation => ActionConfirmationStatus::NotRequired,
        ActionConfirmationRule::ConfirmationRequired if confirmed_by.is_some() => {
            ActionConfirmationStatus::Confirmed
        }
        ActionConfirmationRule::ConfirmationRequired => ActionConfirmationStatus::RequiredPending,
        ActionConfirmationRule::HumanOnly => ActionConfirmationStatus::HumanOnly,
        ActionConfirmationRule::Forbidden => ActionConfirmationStatus::Denied,
    }
}

fn confirmation_status_for_proposal(rule: ActionConfirmationRule) -> ActionConfirmationStatus {
    match rule {
        ActionConfirmationRule::NoConfirmation => ActionConfirmationStatus::NotRequired,
        ActionConfirmationRule::ConfirmationRequired => ActionConfirmationStatus::RequiredPending,
        ActionConfirmationRule::HumanOnly => ActionConfirmationStatus::HumanOnly,
        ActionConfirmationRule::Forbidden => ActionConfirmationStatus::Denied,
    }
}

fn safety_findings(
    request: &HostActionRequest,
    limits: ActionSafetyLimits,
) -> Vec<ActionSafetyFinding> {
    let mut findings = Vec::new();
    if request.attempt.attempts > limits.max_attempts {
        findings.push(ActionSafetyFinding::new(
            "circuit-breaker",
            format!(
                "attempt count {} exceeded max {}",
                request.attempt.attempts, limits.max_attempts
            ),
        ));
    }
    if request.attempt.repeated_action_count >= limits.stuck_loop_warning_after {
        findings.push(ActionSafetyFinding::new(
            "stuck-loop-warning",
            format!(
                "same action repeated {} time(s)",
                request.attempt.repeated_action_count
            ),
        ));
    }
    if request.attempt.repeated_action_count >= limits.circuit_breaker_repeated_actions {
        findings.push(ActionSafetyFinding::new(
            "circuit-breaker",
            format!(
                "same action repeated {} time(s), circuit breaker threshold {}",
                request.attempt.repeated_action_count, limits.circuit_breaker_repeated_actions
            ),
        ));
    }
    if request.attempt.session_age_turns > limits.stale_after_turns {
        findings.push(ActionSafetyFinding::new(
            "stale-session",
            format!(
                "session age {} turn(s) exceeded stale threshold {}",
                request.attempt.session_age_turns, limits.stale_after_turns
            ),
        ));
    }
    if let Some(blocker) = &request.attempt.blocker {
        findings.push(ActionSafetyFinding::new(
            "blocked-needs-human-input",
            blocker.clone(),
        ));
    }
    findings
}

fn default_rollback_hint(kind: HostActionKind) -> String {
    match kind {
        HostActionKind::CreateGitHubIssue => "Close or edit the created issue.".to_owned(),
        HostActionKind::BindGitHubIssue | HostActionKind::BindWorkOrder => {
            "Revert the WorkOrder binding record.".to_owned()
        }
        HostActionKind::RegisterSource | HostActionKind::RefreshSource => {
            "Restore the previous Source Registry entry and pin.".to_owned()
        }
        HostActionKind::BuildContextPack => "Remove or rebuild the ContextPack.".to_owned(),
        HostActionKind::PreparePrEvidenceSummary | HostActionKind::CollectEvidencePacket => {
            "Regenerate the Evidence Packet or PR evidence summary.".to_owned()
        }
        HostActionKind::UpdateTracker => {
            "Revert the tracker checkbox and Evidence Ledger row.".to_owned()
        }
        HostActionKind::OverrideRoleVeto => {
            "Reinstate the veto and document the override.".to_owned()
        }
        HostActionKind::ClaimReleaseReadiness => {
            "Withdraw the release-readiness claim and mark blockers.".to_owned()
        }
        _ => "No persisted change should have occurred.".to_owned(),
    }
}

fn ensure_role(field: &str, value: &str) -> Result<()> {
    ensure_nonempty(field, value)?;
    if value.contains('/') || value.contains('\\') {
        bail!("{field} `{value}` must not contain path separators");
    }
    Ok(())
}

fn ensure_nonempty(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{field} cannot be empty");
    }
    Ok(())
}

fn default_schema_version() -> u32 {
    HOST_ACTION_SCHEMA_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host_request(kind: HostActionKind) -> HostActionRequest {
        HostActionRequest::new(
            "HA-0215",
            "host",
            "host",
            ActionIntent::Execute,
            kind,
            "WO-0215",
            "Exercise host action policy.",
        )
    }

    #[test]
    fn host_classification_is_allowed_without_confirmation() {
        let result =
            evaluate_host_action(host_request(HostActionKind::ClassifyUserIntent)).expect("action");

        assert_eq!(result.decision.outcome, ActionOutcome::Allowed);
        assert_eq!(
            result.decision.confirmation_status,
            ActionConfirmationStatus::NotRequired
        );
        assert!(result.decision.can_execute);
        assert_eq!(result.audit_event.actor_role, "host");
    }

    #[test]
    fn tracker_update_requires_confirmation() {
        let result =
            evaluate_host_action(host_request(HostActionKind::UpdateTracker)).expect("action");

        assert_eq!(result.decision.outcome, ActionOutcome::ConfirmationRequired);
        assert_eq!(
            result.decision.confirmation_status,
            ActionConfirmationStatus::RequiredPending
        );
        assert!(!result.decision.can_execute);
    }

    #[test]
    fn confirmed_tracker_update_is_allowed() {
        let mut request = host_request(HostActionKind::UpdateTracker);
        request.confirmed_by = Some("user".to_owned());

        let result = evaluate_host_action(request).expect("action");

        assert_eq!(result.decision.outcome, ActionOutcome::Allowed);
        assert_eq!(
            result.decision.confirmation_status,
            ActionConfirmationStatus::Confirmed
        );
        assert!(result.decision.can_execute);
    }

    #[test]
    fn specialist_cannot_execute_project_level_action() {
        let request = HostActionRequest::new(
            "HA-0215",
            "engineer",
            "host",
            ActionIntent::Execute,
            HostActionKind::UpdateTracker,
            "#213",
            "Specialist tries to close tracker.",
        );

        let result = evaluate_host_action(request).expect("action");

        assert_eq!(result.decision.outcome, ActionOutcome::Forbidden);
        assert_eq!(result.decision.rule, ActionConfirmationRule::Forbidden);
        assert!(result.decision.reason.contains("non-host"));
    }

    #[test]
    fn specialist_may_propose_project_level_action() {
        let request = HostActionRequest::new(
            "HA-0215",
            "engineer",
            "host",
            ActionIntent::Propose,
            HostActionKind::UpdateTracker,
            "#213",
            "Specialist proposes tracker closure for host review.",
        );

        let result = evaluate_host_action(request).expect("action");

        assert_eq!(result.decision.outcome, ActionOutcome::Proposed);
        assert!(!result.decision.can_execute);
    }

    #[test]
    fn circuit_breaker_blocks_repeated_actions() {
        let mut request = host_request(HostActionKind::UpdateTracker);
        request.attempt.repeated_action_count = 4;
        request.confirmed_by = Some("user".to_owned());

        let result = evaluate_host_action(request).expect("action");

        assert_eq!(result.decision.outcome, ActionOutcome::Failed);
        assert!(result
            .decision
            .safety_findings
            .iter()
            .any(|finding| finding.code == "circuit-breaker"));
    }
}
