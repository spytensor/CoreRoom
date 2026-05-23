//! Conversation visibility rules for the future CoreRoom console.
//!
//! The public conversation defaults to `User <-> @host`. Specialist role
//! collaboration remains host-managed internal delegation unless the user
//! explicitly addressed that role, or `@host` surfaces critical output.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::console_snapshot::ConversationVisibility;

/// Reason `@host` may surface role output in the public transcript.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum HostSurfaceReason {
    /// A role found a critical risk the user must see.
    CriticalRisk,
    /// A role has veto authority and blocked the plan/work.
    Veto,
    /// User confirmation is needed before continuing.
    ConfirmationRequired,
    /// Final evidence summary is being returned to the user.
    FinalEvidenceSummary,
}

impl HostSurfaceReason {
    /// Stable label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::CriticalRisk => "critical-risk",
            Self::Veto => "veto",
            Self::ConfirmationRequired => "confirmation-required",
            Self::FinalEvidenceSummary => "final-evidence-summary",
        }
    }
}

/// Input event category for visibility routing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum ConversationVisibilityInput {
    /// User sent a message.
    UserMessage {
        /// Raw target role if the user explicitly addressed a specialist.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        addressed_role: Option<String>,
    },
    /// Host replied directly to the user.
    HostResponse,
    /// Host delegated work to a role.
    HostToRole {
        /// Target role.
        role: String,
        /// Related WorkOrder.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        work_order: Option<String>,
    },
    /// Role returned output to host from an internal delegation.
    RoleToHost {
        /// Source role.
        role: String,
        /// Whether host is intentionally surfacing this output publicly.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        surfaced_by_host: Option<HostSurfaceReason>,
    },
    /// Role replied directly because the user explicitly addressed it.
    RoleAddressedByUser {
        /// Source role.
        role: String,
    },
    /// Compact side-rail status row.
    SideRailSummary {
        /// Side-rail source label.
        source: String,
    },
    /// Debug/log/Xray event.
    DebugLog {
        /// Debug source label.
        source: String,
    },
}

/// Visibility decision for one conversation-like event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConversationVisibilityDecision {
    /// Surface that should receive the event.
    pub visibility: ConversationVisibility,
    /// Whether the event belongs in the main public transcript.
    pub public_transcript: bool,
    /// Whether the event should update side-rail activity state.
    pub side_rail_activity: bool,
    /// Whether details remain available through logs/Xray.
    pub xray_available: bool,
    /// Human-readable reason for the routing decision.
    pub reason: String,
}

/// Decide where an event belongs.
pub fn decide_visibility(
    input: &ConversationVisibilityInput,
) -> Result<ConversationVisibilityDecision> {
    let decision = match input {
        ConversationVisibilityInput::UserMessage { addressed_role } => {
            if let Some(role) = addressed_role {
                ensure_role(role)?;
                ConversationVisibilityDecision::public(format!("user explicitly addressed @{role}"))
            } else {
                ConversationVisibilityDecision::public(
                    "user message defaults to public transcript".to_owned(),
                )
            }
        }
        ConversationVisibilityInput::HostResponse => ConversationVisibilityDecision::public(
            "@host response defaults to public transcript".to_owned(),
        ),
        ConversationVisibilityInput::HostToRole { role, work_order } => {
            ensure_role(role)?;
            if let Some(work_order) = work_order {
                ensure_work_order_id(work_order)?;
            }
            ConversationVisibilityDecision::internal(format!(
                "@host delegated to @{role}; show compact side-rail activity"
            ))
        }
        ConversationVisibilityInput::RoleToHost {
            role,
            surfaced_by_host,
        } => {
            ensure_role(role)?;
            if let Some(reason) = surfaced_by_host {
                ConversationVisibilityDecision::public(format!(
                    "@host surfaced @{role} output for {}",
                    reason.label()
                ))
            } else {
                ConversationVisibilityDecision::internal(format!(
                    "@{role} replied to @host internal delegation"
                ))
            }
        }
        ConversationVisibilityInput::RoleAddressedByUser { role } => {
            ensure_role(role)?;
            ConversationVisibilityDecision::public(format!(
                "@{role} replied because the user addressed that role"
            ))
        }
        ConversationVisibilityInput::SideRailSummary { source } => {
            ensure_nonempty("sideRail.source", source)?;
            ConversationVisibilityDecision {
                visibility: ConversationVisibility::SideRail,
                public_transcript: false,
                side_rail_activity: true,
                xray_available: true,
                reason: format!("{source} is a compact side-rail summary"),
            }
        }
        ConversationVisibilityInput::DebugLog { source } => {
            ensure_nonempty("debugLog.source", source)?;
            ConversationVisibilityDecision {
                visibility: ConversationVisibility::DebugLog,
                public_transcript: false,
                side_rail_activity: false,
                xray_available: true,
                reason: format!("{source} belongs in debug/log/Xray"),
            }
        }
    };
    Ok(decision)
}

impl ConversationVisibilityDecision {
    fn public(reason: String) -> Self {
        Self {
            visibility: ConversationVisibility::PublicTranscript,
            public_transcript: true,
            side_rail_activity: false,
            xray_available: true,
            reason,
        }
    }

    fn internal(reason: String) -> Self {
        Self {
            visibility: ConversationVisibility::InternalDelegation,
            public_transcript: false,
            side_rail_activity: true,
            xray_available: true,
            reason,
        }
    }
}

fn ensure_role(value: &str) -> Result<()> {
    ensure_nonempty("role", value)?;
    if value.starts_with('@') {
        bail!("role `{value}` must not include leading @");
    }
    Ok(())
}

fn ensure_work_order_id(value: &str) -> Result<()> {
    ensure_nonempty("workOrder", value)?;
    if !value.starts_with("WO-") {
        bail!("WorkOrder id `{value}` must start with `WO-`");
    }
    Ok(())
}

fn ensure_nonempty(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{field} cannot be empty");
    }
    Ok(())
}
