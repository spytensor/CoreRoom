//! Role avatar presentation helpers.
//!
//! Avatars are display-only identity hints. They must never participate in
//! routing, authority, evidence, or completion decisions.

use std::env;

/// Environment variable that selects the role avatar pack.
pub const AVATAR_PACK_ENV: &str = "COREROOM_AVATAR_PACK";

/// Console role avatar pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleAvatarPack {
    /// Terminal-safe geometric glyphs. This is the default.
    Safe,
    /// Nerd Font private-use glyphs. Requires explicit opt-in.
    NerdFont,
}

impl RoleAvatarPack {
    /// Parse a user-facing avatar pack value.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "safe" | "unicode" | "identicon" => Some(Self::Safe),
            "nerd" | "nerd-font" | "nerd-fonts" | "nerdfont" | "nerdfonts" => Some(Self::NerdFont),
            _ => None,
        }
    }

    /// Read avatar pack from the environment, falling back to [`Self::Safe`].
    #[must_use]
    pub fn from_env() -> Self {
        env::var(AVATAR_PACK_ENV)
            .ok()
            .and_then(|value| Self::parse(&value))
            .unwrap_or(Self::Safe)
    }

    /// Stable config/env value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::NerdFont => "nerd-font",
        }
    }
}

/// Display-only role avatar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoleAvatar {
    /// Glyph to render before the role name.
    pub glyph: &'static str,
    /// Pack that produced this glyph.
    pub pack: RoleAvatarPack,
}

/// Return the avatar for a role in the selected pack.
#[must_use]
pub fn role_avatar(role: &str, host_role: &str, pack: RoleAvatarPack) -> RoleAvatar {
    let glyph = match pack {
        RoleAvatarPack::Safe => safe_glyph(role, host_role),
        RoleAvatarPack::NerdFont => nerd_font_glyph(role, host_role),
    };
    RoleAvatar { glyph, pack }
}

/// Format a role label that always preserves the textual role name.
#[must_use]
pub fn role_label(role: &str, host_role: &str, pack: RoleAvatarPack) -> String {
    let avatar = role_avatar(role, host_role, pack);
    format!("{} @{role}", avatar.glyph)
}

fn safe_glyph(role: &str, host_role: &str) -> &'static str {
    if role == host_role || role == "host" {
        return "◉";
    }
    match role {
        "engineer" | "backend" => "◇",
        "reviewer" => "◎",
        "security" => "◆",
        "qa" | "test" | "tester" => "△",
        "sre" | "ops" | "devops" => "▣",
        "frontend" | "design" => "▱",
        "product" | "pm" => "◌",
        _ => fallback_safe_glyph(role),
    }
}

fn nerd_font_glyph(role: &str, host_role: &str) -> &'static str {
    if role == host_role || role == "host" {
        return "󰧑";
    }
    match role {
        "engineer" | "backend" => "󰙨",
        "reviewer" => "󰈈",
        "security" => "󰒃",
        "qa" | "test" | "tester" => "󰙩",
        "sre" | "ops" | "devops" => "󰑮",
        "frontend" | "design" => "󰜈",
        "product" | "pm" => "󰊕",
        _ => fallback_nerd_glyph(role),
    }
}

fn fallback_safe_glyph(role: &str) -> &'static str {
    const GLYPHS: [&str; 8] = ["○", "□", "△", "◇", "◆", "●", "■", "▽"];
    GLYPHS[stable_slot(role)]
}

fn fallback_nerd_glyph(role: &str) -> &'static str {
    const GLYPHS: [&str; 8] = ["󰀄", "󰆧", "󰊢", "󰒋", "󰓾", "󰟵", "󰣇", "󰧮"];
    GLYPHS[stable_slot(role)]
}

fn stable_slot(role: &str) -> usize {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in role.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    (hash as usize) % 8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avatar_pack_parses_aliases_and_defaults_to_safe() {
        assert_eq!(RoleAvatarPack::parse("safe"), Some(RoleAvatarPack::Safe));
        assert_eq!(
            RoleAvatarPack::parse("nerd-fonts"),
            Some(RoleAvatarPack::NerdFont)
        );
        assert_eq!(RoleAvatarPack::parse("wat"), None);
    }

    #[test]
    fn host_role_gets_host_avatar_even_when_renamed() {
        assert_eq!(
            role_avatar("pm", "pm", RoleAvatarPack::Safe).glyph,
            role_avatar("host", "host", RoleAvatarPack::Safe).glyph
        );
        assert_ne!(
            role_avatar("host", "pm", RoleAvatarPack::Safe).glyph,
            role_avatar("engineer", "pm", RoleAvatarPack::Safe).glyph
        );
    }

    #[test]
    fn role_label_preserves_text_identity() {
        let label = role_label("security", "host", RoleAvatarPack::Safe);
        assert!(label.contains("@security"));
        assert!(label.starts_with("◆ "));
    }
}
