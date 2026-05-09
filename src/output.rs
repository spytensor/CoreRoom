//! Centralized output styling.
//!
//! Non-raw-mode user-facing prints in `repl.rs` route through here so the
//! palette and semantic rules live in one place. Raw-mode renderers (the
//! `init` wizard, the in-place thinking spinner, the `UiCell` row builders)
//! consume the constants and fragment helpers but draw their own frames —
//! see `docs/colors.md` §6 / §8 for the carve-outs and rationale.
//!
//! Anything semantic (status messages, role tokens, command tokens, tool
//! traces) should call into this module rather than reaching for a raw
//! `crossterm` color. That way swapping a palette entry later touches one
//! file instead of forty.

use std::io::IsTerminal;

use crossterm::style::{Color, StyledContent, Stylize};

// ───────────────────── semantic colors ─────────────────────

/// Success — paired with `✓`.
pub const OK: Color = Color::Rgb {
    r: 0x97,
    g: 0xc4,
    b: 0x59,
};
/// Attention but not failure — paired with `⚠`, `⟳`, `⊘`.
pub const WARN: Color = Color::Rgb {
    r: 0xef,
    g: 0x9f,
    b: 0x27,
};
/// Failure — paired with `✗`.
pub const BAD: Color = Color::Rgb {
    r: 0xea,
    g: 0x5b,
    b: 0x5b,
};
/// Neutral hint, auto-routing arrow.
pub const INFO: Color = Color::Rgb {
    r: 0x6f,
    g: 0xa8,
    b: 0xdc,
};
/// Commands and hotkeys (`/help`, `cr update`).
pub const KEY: Color = Color::Rgb {
    r: 0xd4,
    g: 0xb8,
    b: 0x7a,
};
/// Input prompt (`cr ›`).
pub const PROMPT: Color = Color::Rgb {
    r: 0x58,
    g: 0xc3,
    b: 0x9c,
};
/// Emphasis: API paths, key values.
pub const EM: Color = Color::Rgb {
    r: 0xf0,
    g: 0xf0,
    b: 0xf0,
};
/// Default body text.
pub const TEXT: Color = Color::Rgb {
    r: 0xd4,
    g: 0xd4,
    b: 0xd4,
};
/// Secondary information: timestamps, side labels.
pub const MUTE: Color = Color::Rgb {
    r: 0x9a,
    g: 0x9a,
    b: 0x9a,
};
/// System rows: tool call summaries, in-place spinner status.
pub const DIM: Color = Color::Rgb {
    r: 0x82,
    g: 0x82,
    b: 0x82,
};
/// Decorative `·` separators and the `↳` glyph. Sub-AA by design.
pub const FADE: Color = Color::Rgb {
    r: 0x4a,
    g: 0x4a,
    b: 0x4a,
};
/// Box borders. Reserved for future drawing; sub-AA by design.
pub const RULE: Color = Color::Rgb {
    r: 0x3a,
    g: 0x3a,
    b: 0x3d,
};

// ───────────────────── role palette ────────────────────────

const ROLE_PALETTE: [Color; 8] = [
    // 0: lavender — `@host` is pinned here.
    Color::Rgb {
        r: 0xc0,
        g: 0xa8,
        b: 0xff,
    },
    // 1: jade
    Color::Rgb {
        r: 0x5d,
        g: 0xca,
        b: 0xa5,
    },
    // 2: coral
    Color::Rgb {
        r: 0xf0,
        g: 0x99,
        b: 0x7b,
    },
    // 3: rose
    Color::Rgb {
        r: 0xf0,
        g: 0x90,
        b: 0x80,
    },
    // 4: sky
    Color::Rgb {
        r: 0x85,
        g: 0xb7,
        b: 0xeb,
    },
    // 5: blossom
    Color::Rgb {
        r: 0xe0,
        g: 0x88,
        b: 0xc4,
    },
    // 6: honey
    Color::Rgb {
        r: 0xf4,
        g: 0xc7,
        b: 0x75,
    },
    // 7: teal
    Color::Rgb {
        r: 0x7b,
        g: 0xc6,
        b: 0xc1,
    },
];

/// FNV-1a 32-bit. Stable across Rust toolchain versions, unlike
/// `std::hash::DefaultHasher`. Five lines, no dependency.
fn fnv1a(s: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in s.as_bytes() {
        h ^= u32::from(*b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// Stable role color. The host role is always lavender; every other role
/// hashes deterministically into slots 1..8. The same role name therefore
/// keeps the same color across sessions and across Rust versions.
#[must_use]
pub fn role_color(role: &str, host_role: &str) -> Color {
    if role == host_role {
        return ROLE_PALETTE[0];
    }
    let idx = 1 + (fnv1a(role) as usize) % 7;
    ROLE_PALETTE[idx]
}

// ───────────────────── status helpers ──────────────────────

/// `✓ <msg>` in success colors.
pub fn ok(msg: impl AsRef<str>) {
    println!("{} {}", "✓".with(OK), msg.as_ref().with(TEXT));
}

/// `⚠ <msg>` in warning colors.
pub fn warn(msg: impl AsRef<str>) {
    println!("{} {}", "⚠".with(WARN), msg.as_ref().with(TEXT));
}

/// `✗ <msg>` in failure colors.
pub fn bad(msg: impl AsRef<str>) {
    println!("{} {}", "✗".with(BAD), msg.as_ref().with(TEXT));
}

/// Indented secondary line that follows a primary status line. Color
/// steps down to `FADE` per the npm/cargo convention.
pub fn hint(msg: impl AsRef<str>) {
    println!("  {}", msg.as_ref().with(FADE));
}

/// `[<msg>]` in dimmed italic — system bracket convention used for
/// `[@role ready · model=...]` and `[@role stopped: ...]`.
pub fn system(msg: impl AsRef<str>) {
    println!("{}", format!("[{}]", msg.as_ref()).with(DIM).italic());
}

/// `  ↳ @role · <summary>` — tool-call trace. No timestamp by design;
/// tool events are too frequent for a per-line clock.
pub fn tool_trace(role: &str, summary: impl AsRef<str>) {
    println!(
        "  {} @{role} · {}",
        "↳".with(FADE),
        summary.as_ref().with(DIM),
    );
}

// ───────────────────── fragment helpers ────────────────────
//
// The `repl.rs` home dashboard builds rows out of `UiCell` values that
// pair a styled string with its visible character count. These helpers
// return styled fragments so those builders can continue to assemble
// their own lines without re-importing the palette directly.

/// `@<role>` styled with the role's color and bold.
#[must_use]
pub fn role_token(role: &str, host_role: &str) -> StyledContent<String> {
    format!("@{role}").with(role_color(role, host_role)).bold()
}

/// `●` styled with the role's color (used in the boot role list).
#[must_use]
pub fn role_dot(role: &str, host_role: &str) -> StyledContent<&'static str> {
    "●".with(role_color(role, host_role))
}

/// Style a command/hotkey token (`/help`, `cr update`).
#[must_use]
pub fn cmd(text: impl Into<String>) -> StyledContent<String> {
    text.into().with(KEY)
}

/// The `cr ›` prompt as a string. The caller controls placement and
/// flushing — this helper just owns the styling.
#[must_use]
pub fn prompt() -> String {
    format!("\n{} ", "cr ›".with(PROMPT).bold())
}

// ───────────────────── typography helpers ─────────────────

/// Truncate `s` so it occupies at most `max_chars` visible cells,
/// appending `…` when truncation happens. Counts via `chars().count()`,
/// which matches the visible width for ASCII identifiers and the
/// glyphs used elsewhere in this module. Wide-East-Asian descriptions
/// would need `unicode-width`, but role names are ASCII-constrained at
/// validation time.
#[must_use]
pub fn truncate_visible(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let len = s.chars().count();
    if len <= max_chars {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

// ───────────────────── degradation ─────────────────────────

/// Whether colored output is appropriate in this process. v0.1 relies on
/// crossterm's own `should_colorize` behaviour for TTY detection; explicit
/// `NO_COLOR` plumbing is v0.1.x.
#[must_use]
pub fn color_enabled() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    if !std::io::stdout().is_terminal() {
        return false;
    }
    !matches!(std::env::var("TERM").as_deref(), Ok("dumb"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_role_pins_to_lavender() {
        let host = role_color("host", "host");
        assert_eq!(host, ROLE_PALETTE[0]);
    }

    #[test]
    fn role_color_is_stable_for_same_name() {
        // Same name ⇒ same color across calls.
        assert_eq!(role_color("backend", "host"), role_color("backend", "host"));
        assert_eq!(
            role_color("frontend", "host"),
            role_color("frontend", "host")
        );
    }

    #[test]
    fn role_color_differs_when_a_role_is_promoted_to_host() {
        // `backend` as a regular role hashes into slots 1..8.
        // Once `backend` becomes the host, it pins to lavender.
        let regular = role_color("backend", "host");
        let as_host = role_color("backend", "backend");
        assert_eq!(as_host, ROLE_PALETTE[0]);
        assert_ne!(regular, as_host);
    }

    #[test]
    fn fnv1a_matches_known_values() {
        // Public FNV-1a 32-bit reference vectors.
        assert_eq!(fnv1a(""), 0x811c_9dc5);
        assert_eq!(fnv1a("a"), 0xe40c_292c);
        assert_eq!(fnv1a("foobar"), 0xbf9c_f968);
    }

    #[test]
    fn non_host_roles_never_use_lavender() {
        // Slot 0 is reserved; only the host can land there.
        for name in [
            "backend", "frontend", "qa", "devops", "data", "security", "docs", "auth", "ingest",
            "platform", "ml", "infra",
        ] {
            assert_ne!(
                role_color(name, "host"),
                ROLE_PALETTE[0],
                "role `{name}` collided with the host slot"
            );
        }
    }
}
