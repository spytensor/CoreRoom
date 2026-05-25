//! ratatui-side styling helpers for role identity.
//!
//! [`crate::output::role_color`] is the source of truth for role colors, but
//! it returns a `crossterm::style::Color`. The live-room TUI draws with
//! ratatui types, so this module bridges crossterm → ratatui and pairs the
//! color with the role's avatar glyph from [`crate::role_avatar`].
//!
//! Every ratatui surface that needs to render a role label should call
//! [`role_label_spans`] so identity stays consistent across splash,
//! spinner rows, work cards, and the composer candidate menu.

use crossterm::style::Color as CrosstermColor;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

use crate::output;
use crate::role_avatar::{role_avatar, RoleAvatarPack};

/// Convert a `crossterm::style::Color` to its `ratatui::style::Color`
/// equivalent. RGB values are preserved exactly; ANSI 256-colors map
/// through [`Color::Indexed`]; named colors map 1:1.
#[must_use]
pub fn crossterm_to_ratatui_color(color: CrosstermColor) -> Color {
    match color {
        CrosstermColor::Reset => Color::Reset,
        CrosstermColor::Black => Color::Black,
        CrosstermColor::DarkGrey => Color::DarkGray,
        CrosstermColor::Red => Color::LightRed,
        CrosstermColor::DarkRed => Color::Red,
        CrosstermColor::Green => Color::LightGreen,
        CrosstermColor::DarkGreen => Color::Green,
        CrosstermColor::Yellow => Color::LightYellow,
        CrosstermColor::DarkYellow => Color::Yellow,
        CrosstermColor::Blue => Color::LightBlue,
        CrosstermColor::DarkBlue => Color::Blue,
        CrosstermColor::Magenta => Color::LightMagenta,
        CrosstermColor::DarkMagenta => Color::Magenta,
        CrosstermColor::Cyan => Color::LightCyan,
        CrosstermColor::DarkCyan => Color::Cyan,
        CrosstermColor::White => Color::White,
        CrosstermColor::Grey => Color::Gray,
        CrosstermColor::Rgb { r, g, b } => Color::Rgb(r, g, b),
        CrosstermColor::AnsiValue(i) => Color::Indexed(i),
    }
}

/// Stable role color in ratatui form. Mirrors [`output::role_color`].
#[must_use]
pub fn role_color(role: &str, host_role: &str) -> Color {
    crossterm_to_ratatui_color(output::role_color(role, host_role))
}

/// Avatar glyph for a role using the user-selected (env) pack.
#[must_use]
pub fn role_avatar_glyph(role: &str, host_role: &str) -> &'static str {
    role_avatar(role, host_role, RoleAvatarPack::from_env()).glyph
}

/// Build styled spans for a role label: `{glyph} @{role}` with the glyph
/// colored to the role's identity and the role token colored + bold.
///
/// This is the single helper every ratatui surface should call to render
/// a role label. Sub-components that need to colorize text other than the
/// label itself (a spinner frame, a work-card title body) can call
/// [`role_color`] or [`role_avatar_glyph`] directly.
#[must_use]
pub fn role_label_spans(role: &str, host_role: &str) -> Vec<Span<'static>> {
    let color = role_color(role, host_role);
    let glyph = role_avatar_glyph(role, host_role);
    vec![
        Span::styled(glyph.to_owned(), Style::default().fg(color)),
        Span::raw(" "),
        Span::styled(
            format!("@{role}"),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_components_are_preserved() {
        let c = CrosstermColor::Rgb {
            r: 0xff,
            g: 0x88,
            b: 0x66,
        };
        assert_eq!(crossterm_to_ratatui_color(c), Color::Rgb(0xff, 0x88, 0x66));
    }

    #[test]
    fn named_colors_map_one_to_one() {
        assert_eq!(
            crossterm_to_ratatui_color(CrosstermColor::Reset),
            Color::Reset
        );
        assert_eq!(
            crossterm_to_ratatui_color(CrosstermColor::DarkGrey),
            Color::DarkGray
        );
    }

    #[test]
    fn role_color_matches_crossterm_source_of_truth() {
        for role in ["host", "backend", "engineer", "reviewer", "frontend"] {
            assert_eq!(
                role_color(role, "host"),
                crossterm_to_ratatui_color(output::role_color(role, "host"))
            );
        }
    }

    #[test]
    fn host_pins_to_lavender_via_ratatui_bridge() {
        let host_ratatui = role_color("host", "host");
        // Lavender = palette slot 0, defined as RGB(0xb8, 0x9c, 0xff).
        assert_eq!(host_ratatui, Color::Rgb(0xb8, 0x9c, 0xff));
    }

    #[test]
    fn promoted_role_inherits_host_color() {
        let regular = role_color("backend", "host");
        let promoted = role_color("backend", "backend");
        let host = role_color("host", "host");
        assert_eq!(promoted, host);
        assert_ne!(promoted, regular);
    }

    #[test]
    fn canonical_roles_get_distinct_colors() {
        let backend = role_color("backend", "host");
        let reviewer = role_color("reviewer", "host");
        let security = role_color("security", "host");
        let frontend = role_color("frontend", "host");
        let host = role_color("host", "host");
        let all = [backend, reviewer, security, frontend, host];
        // No two should collide.
        for i in 0..all.len() {
            for j in (i + 1)..all.len() {
                assert_ne!(all[i], all[j], "colors {i} and {j} collide");
            }
        }
    }

    #[test]
    fn role_label_spans_has_glyph_space_token_layout() {
        let spans = role_label_spans("backend", "host");
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[1].content.as_ref(), " ");
        assert_eq!(spans[2].content.as_ref(), "@backend");
    }

    #[test]
    fn role_label_spans_glyph_and_token_share_color() {
        let spans = role_label_spans("backend", "host");
        let g_fg = spans[0].style.fg;
        let t_fg = spans[2].style.fg;
        assert!(g_fg.is_some());
        assert_eq!(g_fg, t_fg);
    }

    #[test]
    fn role_label_spans_token_is_bold() {
        let spans = role_label_spans("backend", "host");
        assert!(spans[2].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn role_color_is_deterministic_for_unknown_roles() {
        let a = role_color("ingestor", "host");
        let b = role_color("ingestor", "host");
        assert_eq!(a, b);
    }

    #[test]
    fn role_label_spans_uses_safe_glyph_by_default() {
        // Default env pack is Safe. Host role glyph is ◉.
        let spans = role_label_spans("host", "host");
        assert_eq!(spans[0].content.as_ref(), "◉");
    }
}
