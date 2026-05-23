//! Public conversation projection for the full-screen console.
//!
//! The center panel is intentionally narrow in authority: user and `@host`
//! remain primary, while side-rail/internal delegation stays out of the public
//! transcript unless it was already surfaced as a public turn.

use crate::console_snapshot::{
    ConversationTurn, ConversationVisibility, CoreRoomSnapshot, InternalDelegationActivity,
};

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
