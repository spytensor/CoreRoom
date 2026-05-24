//! Public conversation projection for the full-screen console.
//!
//! The center panel is intentionally narrow in authority: user and `@host`
//! remain primary, while side-rail/internal delegation stays out of the public
//! transcript unless it was already surfaced as a public turn.

use crate::console_snapshot::{
    ConversationTurn, ConversationVisibility, CoreRoomSnapshot, InternalDelegationActivity,
    InternalDelegationState,
};

/// What authority the conversation surface has over engineering state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationAuthority {
    /// The conversation is display-only. Completion/evidence must come from
    /// structured WorkOrders, gates, Evidence Packets, CI, and tracker facts.
    DisplayOnly,
}

/// Live room conversation panel rendered in the console center.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveRoomConversationPanel {
    /// Public turns visible in the main conversation.
    pub public_turns: Vec<LiveRoomTurn>,
    /// Internal host-managed delegation cards rendered outside public chat.
    pub task_cards: Vec<InternalTaskCard>,
    /// Internal delegation count from the snapshot.
    pub hidden_internal_count: u32,
    /// Public-turn entries folded out of chat into the side rail.
    pub side_rail_turn_count: usize,
    /// Authority boundary for this view.
    pub authority: ConversationAuthority,
}

impl LiveRoomConversationPanel {
    /// Whether a non-host specialist role is visible because the snapshot made
    /// that turn public.
    #[must_use]
    pub fn has_public_specialist_turn(&self) -> bool {
        self.public_turns
            .iter()
            .any(|turn| turn.kind == LiveRoomTurnKind::DirectSpecialist)
    }
}

/// One public live room turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveRoomTurn {
    /// Speaker label such as `user`, `host`, or a role.
    pub speaker: String,
    /// Turn body.
    pub body: String,
    /// Conversation role for rendering and visibility.
    pub kind: LiveRoomTurnKind,
}

/// Public turn category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveRoomTurnKind {
    /// User-originated prompt.
    User,
    /// Configured host response.
    Host,
    /// Specialist response that is public only because the user addressed it
    /// or host intentionally surfaced it.
    DirectSpecialist,
}

/// Compact card for host-managed internal delegation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InternalTaskCard {
    /// Delegated role.
    pub role: String,
    /// Related WorkOrder, if known.
    pub work_order: Option<String>,
    /// Current internal task state.
    pub state: InternalDelegationState,
    /// Compact user-facing summary.
    pub summary: String,
    /// Detail/Xray reference.
    pub xray_ref: Option<String>,
}

/// Public conversation panel rendered in the console center.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicConversationPanel {
    /// Turns visible in the public transcript.
    pub turns: Vec<ConversationTurn>,
    /// Internal delegation count from the snapshot.
    pub hidden_internal_count: u32,
    /// Public-turn entries folded out of chat into the side rail.
    pub side_rail_turn_count: usize,
    /// Internal delegation activity rows available to side rails/Xray.
    pub internal_activity: Vec<InternalDelegationActivity>,
}

impl PublicConversationPanel {
    /// Whether a non-host specialist role is visible because the snapshot made
    /// that turn public.
    #[must_use]
    pub fn has_public_specialist_turn(&self, host_role: &str) -> bool {
        self.turns.iter().any(|turn| {
            turn.speaker != "user" && turn.speaker != host_role && turn.speaker != "host"
        })
    }
}

/// Build the public conversation panel from snapshot conversation facts.
#[must_use]
pub fn build_public_conversation(snapshot: &CoreRoomSnapshot) -> PublicConversationPanel {
    let turns = snapshot
        .conversation
        .public_turns
        .iter()
        .filter(|turn| turn.visibility == ConversationVisibility::PublicTranscript)
        .cloned()
        .collect();
    let side_rail_turn_count = snapshot
        .conversation
        .public_turns
        .iter()
        .filter(|turn| turn.visibility == ConversationVisibility::SideRail)
        .count();
    PublicConversationPanel {
        turns,
        hidden_internal_count: snapshot.conversation.internal_delegation_count,
        side_rail_turn_count,
        internal_activity: snapshot.conversation.internal_activity.clone(),
    }
}

/// Build the live room conversation model from snapshot conversation facts.
///
/// This is a display model only. It deliberately carries no completion,
/// approval, release, CI, or evidence-closure state.
#[must_use]
pub fn build_live_room_conversation(snapshot: &CoreRoomSnapshot) -> LiveRoomConversationPanel {
    let public_turns = snapshot
        .conversation
        .public_turns
        .iter()
        .filter(|turn| turn.visibility == ConversationVisibility::PublicTranscript)
        .map(|turn| LiveRoomTurn {
            speaker: turn.speaker.clone(),
            body: turn.body.clone(),
            kind: turn_kind(&turn.speaker, &snapshot.runtime.host_role),
        })
        .collect();
    let side_rail_turn_count = snapshot
        .conversation
        .public_turns
        .iter()
        .filter(|turn| turn.visibility == ConversationVisibility::SideRail)
        .count();
    let task_cards = snapshot
        .conversation
        .internal_activity
        .iter()
        .map(task_card_from_activity)
        .collect();
    LiveRoomConversationPanel {
        public_turns,
        task_cards,
        hidden_internal_count: snapshot.conversation.internal_delegation_count,
        side_rail_turn_count,
        authority: ConversationAuthority::DisplayOnly,
    }
}

fn turn_kind(speaker: &str, host_role: &str) -> LiveRoomTurnKind {
    if speaker == "user" {
        LiveRoomTurnKind::User
    } else if speaker == host_role || speaker == "host" {
        LiveRoomTurnKind::Host
    } else {
        LiveRoomTurnKind::DirectSpecialist
    }
}

fn task_card_from_activity(activity: &InternalDelegationActivity) -> InternalTaskCard {
    InternalTaskCard {
        role: activity.role.clone(),
        work_order: activity.work_order.clone(),
        state: activity.state,
        summary: activity.summary.clone(),
        xray_ref: activity.xray_ref.clone(),
    }
}
