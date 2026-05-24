//! Live room bridge for the unified console path.
//!
//! This module owns the bridge between the renderer-independent composer and
//! existing REPL routing semantics. It does not spawn role engines itself and
//! does not treat rendered conversation prose as evidence.

use anyhow::Result;

use crate::console_actions::ConsolePermissionOverlay;
use crate::console_composer::ComposerCommandSpec;
use crate::console_snapshot::{
    ConversationTurn, ConversationVisibility, CoreRoomSnapshot, RoleLaneState,
};
use crate::repl::{parse_line, Command, PermissionCommand};

/// Stable composer command specs for the live room path.
#[must_use]
pub fn live_room_command_specs() -> Vec<ComposerCommandSpec> {
    vec![
        ComposerCommandSpec::new("allow", "allow a tool for the session", true),
        ComposerCommandSpec::new("deny", "deny a tool for the session", true),
        ComposerCommandSpec::new("exit", "leave the live room", false),
        ComposerCommandSpec::new("fresh", "restart roles cleanly", false),
        ComposerCommandSpec::new("halt", "interrupt current turn", false),
        ComposerCommandSpec::new("help", "show help", false),
        ComposerCommandSpec::new("host", "swap host role for this session", true),
        ComposerCommandSpec::new("permissions", "show session tool approvals", false),
        ComposerCommandSpec::new("quit", "leave the live room", false),
        ComposerCommandSpec::new("refresh", "refresh a role", true),
    ]
}

/// State for one live room bridge session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveRoomBridge {
    host_role: String,
    roles: Vec<String>,
    last_action: Option<LiveRoomAction>,
    permission_overlay: Option<ConsolePermissionOverlay>,
}

impl LiveRoomBridge {
    /// Build a bridge from the current snapshot facts.
    #[must_use]
    pub fn from_snapshot(snapshot: &CoreRoomSnapshot) -> Self {
        let mut roles = snapshot
            .runtime
            .roles
            .iter()
            .filter(|role| role.enabled)
            .map(|role| role.role.clone())
            .collect::<Vec<_>>();
        roles.sort();
        Self {
            host_role: snapshot.runtime.host_role.clone(),
            roles,
            last_action: None,
            permission_overlay: None,
        }
    }

    /// Configured host role.
    #[must_use]
    pub fn host_role(&self) -> &str {
        &self.host_role
    }

    /// Enabled role names.
    #[must_use]
    pub fn roles(&self) -> &[String] {
        &self.roles
    }

    /// Last routed action.
    #[must_use]
    pub const fn last_action(&self) -> Option<&LiveRoomAction> {
        self.last_action.as_ref()
    }

    /// Pending permission/action overlay.
    #[must_use]
    pub const fn permission_overlay(&self) -> Option<&ConsolePermissionOverlay> {
        self.permission_overlay.as_ref()
    }

    /// Surface a permission/action overlay without mutating snapshot facts.
    pub fn set_permission_overlay(&mut self, overlay: ConsolePermissionOverlay) {
        self.permission_overlay = Some(overlay);
    }

    /// Clear a pending permission/action overlay.
    pub fn clear_permission_overlay(&mut self) {
        self.permission_overlay = None;
    }

    /// Submit one composer buffer through existing REPL parse semantics.
    pub fn submit(
        &mut self,
        snapshot: &mut CoreRoomSnapshot,
        input: &str,
    ) -> Result<LiveRoomAction> {
        let action = self.route(input);
        apply_action_to_snapshot(snapshot, &action);
        Self::update_role_lanes(snapshot, &action);
        self.last_action = Some(action.clone());
        Ok(action)
    }

    fn route(&self, input: &str) -> LiveRoomAction {
        match parse_line(input) {
            Command::Empty => LiveRoomAction::Noop,
            Command::SendToHost(text) => LiveRoomAction::Dispatch {
                target_role: self.host_role.clone(),
                text,
                origin: DispatchOrigin::BareUserText,
            },
            Command::SendTo { role, text } => LiveRoomAction::Dispatch {
                target_role: role,
                text,
                origin: DispatchOrigin::ExplicitRoleMention,
            },
            Command::Broadcast(text) => LiveRoomAction::Broadcast { text },
            Command::Exit => LiveRoomAction::Exit,
            Command::Help => LiveRoomAction::SupportedSlash {
                command: "help".to_owned(),
                message: live_room_help_message(),
            },
            Command::Halt(target) => LiveRoomAction::SupportedSlash {
                command: "halt".to_owned(),
                message: match target {
                    Some(role) => format!("live room bridge would halt @{role}"),
                    None => "live room bridge would halt current turns".to_owned(),
                },
            },
            Command::Fresh => LiveRoomAction::SupportedSlash {
                command: "fresh".to_owned(),
                message: "live room bridge would request fresh role sessions".to_owned(),
            },
            Command::Refresh(role) => LiveRoomAction::SupportedSlash {
                command: "refresh".to_owned(),
                message: format!("live room bridge would refresh @{role}"),
            },
            Command::Permissions(command) => LiveRoomAction::SupportedSlash {
                command: "permissions".to_owned(),
                message: match command {
                    PermissionCommand::Show => {
                        "live room bridge would show session permission policy".to_owned()
                    }
                    PermissionCommand::Clear => {
                        "live room bridge would clear session permission policy".to_owned()
                    }
                },
            },
            Command::Allow(tool) => LiveRoomAction::SupportedSlash {
                command: "allow".to_owned(),
                message: format!("live room bridge would allow `{tool}` for this session"),
            },
            Command::Deny(tool) => LiveRoomAction::SupportedSlash {
                command: "deny".to_owned(),
                message: format!("live room bridge would deny `{tool}` for this session"),
            },
            Command::Host(role) => LiveRoomAction::SupportedSlash {
                command: "host".to_owned(),
                message: format!("live room bridge would make @{role} the session host"),
            },
            Command::Patch { .. }
            | Command::Compact(_)
            | Command::Stop(_)
            | Command::Resume(_)
            | Command::Transcript(_)
            | Command::Journal(_)
            | Command::Welcome => LiveRoomAction::UnsupportedSlash {
                command: slash_name(input),
                message: "not yet available in the unified room; use `cr start` for this legacy REPL command while runtime parity closes".to_owned(),
            },
        }
    }

    fn update_role_lanes(snapshot: &mut CoreRoomSnapshot, action: &LiveRoomAction) {
        let LiveRoomAction::Dispatch { target_role, .. } = action else {
            return;
        };
        snapshot.runtime.active_role = Some(target_role.clone());
        for role in &mut snapshot.runtime.roles {
            role.state = if role.role == *target_role {
                RoleLaneState::Working
            } else {
                RoleLaneState::Idle
            };
            if role.role == *target_role {
                role.last_activity = Some("staged preview route".to_owned());
            }
        }
    }
}

/// One action produced by the live room bridge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveRoomAction {
    /// Nothing submitted.
    Noop,
    /// Dispatch prompt text to one role.
    Dispatch {
        /// Target role.
        target_role: String,
        /// Prompt text.
        text: String,
        /// Dispatch origin.
        origin: DispatchOrigin,
    },
    /// Broadcast prompt text to all roles.
    Broadcast {
        /// Prompt text.
        text: String,
    },
    /// Supported slash command handled or staged by the bridge.
    SupportedSlash {
        /// Command name without slash.
        command: String,
        /// Clear user-facing message.
        message: String,
    },
    /// Slash command that still belongs to the old REPL path.
    UnsupportedSlash {
        /// Command name without slash.
        command: String,
        /// Clear user-facing message.
        message: String,
    },
    /// Exit the live room.
    Exit,
}

impl LiveRoomAction {
    /// User-facing status line.
    #[must_use]
    pub fn status_line(&self) -> String {
        match self {
            Self::Noop => "no input submitted".to_owned(),
            Self::Dispatch {
                target_role,
                origin,
                ..
            } => format!("preview-staged for @{target_role} via {}", origin.label()),
            Self::Broadcast { .. } => "broadcast preview-staged for all roles".to_owned(),
            Self::SupportedSlash { message, .. } | Self::UnsupportedSlash { message, .. } => {
                message.clone()
            }
            Self::Exit => "exit requested".to_owned(),
        }
    }
}

/// Origin of a role dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchOrigin {
    /// Bare text routes to configured host.
    BareUserText,
    /// User explicitly addressed a role.
    ExplicitRoleMention,
}

impl DispatchOrigin {
    const fn label(self) -> &'static str {
        match self {
            Self::BareUserText => "bare user text",
            Self::ExplicitRoleMention => "explicit @role mention",
        }
    }
}

fn apply_action_to_snapshot(snapshot: &mut CoreRoomSnapshot, action: &LiveRoomAction) {
    match action {
        LiveRoomAction::Dispatch {
            target_role,
            text,
            origin,
        } => {
            let body = match origin {
                DispatchOrigin::BareUserText => text.clone(),
                DispatchOrigin::ExplicitRoleMention => format!("@{target_role} {text}"),
            };
            snapshot.conversation.public_turns.push(ConversationTurn {
                speaker: "user".to_owned(),
                body,
                visibility: ConversationVisibility::PublicTranscript,
            });
        }
        LiveRoomAction::Broadcast { text } => {
            snapshot.conversation.public_turns.push(ConversationTurn {
                speaker: "user".to_owned(),
                body: format!("@all {text}"),
                visibility: ConversationVisibility::PublicTranscript,
            });
        }
        LiveRoomAction::SupportedSlash { message, .. }
        | LiveRoomAction::UnsupportedSlash { message, .. } => {
            snapshot.conversation.public_turns.push(ConversationTurn {
                speaker: snapshot.runtime.host_role.clone(),
                body: message.clone(),
                visibility: ConversationVisibility::PublicTranscript,
            });
        }
        LiveRoomAction::Noop | LiveRoomAction::Exit => {}
    }
}

fn live_room_help_message() -> String {
    "live room preview supports bare text, explicit @role tasks, @all, /help, and /exit as staged routing; use plain `cr` or `cr start` for real role-engine execution until runtime parity closes".to_owned()
}

fn slash_name(input: &str) -> String {
    input
        .trim_start()
        .strip_prefix('/')
        .and_then(|rest| rest.split_whitespace().next())
        .filter(|name| !name.is_empty())
        .unwrap_or("unknown")
        .to_owned()
}
