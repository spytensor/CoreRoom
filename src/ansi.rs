//! Minimal SGR (Select Graphic Rendition) parser that turns a crossterm
//! styled string into a ratatui [`Line`] with colored [`Span`]s.
//!
//! Why we need this: the executable live-room TUI receives splash and
//! CREP output as already-rendered ANSI strings (because the same
//! string is emitted to plain stdout by `cr start`). Pushing those
//! strings into the ratatui scrollback as `Line::raw` after
//! `strip_ansi` loses every color crossterm encoded. Parsing the SGR
//! codes back into ratatui [`Style`]s preserves role identity colors,
//! frame strokes, and inline status colors across the boundary.
//!
//! This parser is intentionally small and only handles the SGR codes
//! crossterm actually emits via [`crossterm::style::Stylize`]:
//! 24-bit RGB foreground/background (`38;2;…` / `48;2;…`), 256-color
//! indexed (`38;5;…` / `48;5;…`), bold (`1` / `22`), italic
//! (`3` / `23`), reset fg/bg (`39` / `49`), and full reset (`0`).
//! Non-SGR escape sequences are stripped silently. Carriage returns
//! are dropped.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Parse one logical line (no embedded `\n`) into a styled [`Line`].
#[must_use]
pub fn ansi_to_line(input: &str) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut current_style = Style::default();
    let mut buffer = String::new();
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            let mut params = String::new();
            let mut terminator: Option<char> = None;
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    terminator = Some(next);
                    break;
                }
                params.push(next);
            }
            if terminator == Some('m') {
                if !buffer.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut buffer), current_style));
                }
                current_style = apply_sgr(current_style, &params);
            }
            // Non-SGR escapes (cursor moves, clears, etc.) are dropped.
            continue;
        }
        if c == '\r' {
            continue;
        }
        buffer.push(c);
    }
    if !buffer.is_empty() {
        spans.push(Span::styled(buffer, current_style));
    }
    Line::from(spans)
}

/// Split `text` on `\n`, parsing each line with [`ansi_to_line`].
/// Trailing carriage returns are dropped to match crossterm's output
/// in cooked terminals.
#[must_use]
pub fn ansi_to_lines(text: &str) -> Vec<Line<'static>> {
    text.lines().map(ansi_to_line).collect()
}

fn apply_sgr(mut style: Style, params: &str) -> Style {
    if params.is_empty() {
        return Style::default();
    }
    let nums: Vec<u16> = params
        .split(';')
        .filter_map(|s| s.parse::<u16>().ok())
        .collect();
    let mut i = 0;
    while i < nums.len() {
        match nums[i] {
            0 => style = Style::default(),
            1 => style = style.add_modifier(Modifier::BOLD),
            3 => style = style.add_modifier(Modifier::ITALIC),
            4 => style = style.add_modifier(Modifier::UNDERLINED),
            22 => style = style.remove_modifier(Modifier::BOLD),
            23 => style = style.remove_modifier(Modifier::ITALIC),
            24 => style = style.remove_modifier(Modifier::UNDERLINED),
            38 if nums.get(i + 1) == Some(&2) && nums.len() >= i + 5 => {
                style = style.fg(Color::Rgb(
                    to_u8(nums[i + 2]),
                    to_u8(nums[i + 3]),
                    to_u8(nums[i + 4]),
                ));
                i += 4;
            }
            48 if nums.get(i + 1) == Some(&2) && nums.len() >= i + 5 => {
                style = style.bg(Color::Rgb(
                    to_u8(nums[i + 2]),
                    to_u8(nums[i + 3]),
                    to_u8(nums[i + 4]),
                ));
                i += 4;
            }
            38 if nums.get(i + 1) == Some(&5) && nums.len() >= i + 3 => {
                style = style.fg(Color::Indexed(to_u8(nums[i + 2])));
                i += 2;
            }
            48 if nums.get(i + 1) == Some(&5) && nums.len() >= i + 3 => {
                style = style.bg(Color::Indexed(to_u8(nums[i + 2])));
                i += 2;
            }
            39 => style = style.fg(Color::Reset),
            49 => style = style.bg(Color::Reset),
            // Basic 30-37 / 90-97 named-color foregrounds.
            30..=37 => style = style.fg(named_color(nums[i] - 30)),
            90..=97 => style = style.fg(bright_color(nums[i] - 90)),
            40..=47 => style = style.bg(named_color(nums[i] - 40)),
            100..=107 => style = style.bg(bright_color(nums[i] - 100)),
            _ => {}
        }
        i += 1;
    }
    style
}

fn to_u8(value: u16) -> u8 {
    u8::try_from(value).unwrap_or(u8::MAX)
}

fn named_color(idx: u16) -> Color {
    match idx {
        0 => Color::Black,
        1 => Color::Red,
        2 => Color::Green,
        3 => Color::Yellow,
        4 => Color::Blue,
        5 => Color::Magenta,
        6 => Color::Cyan,
        7 => Color::Gray,
        _ => Color::Reset,
    }
}

fn bright_color(idx: u16) -> Color {
    match idx {
        0 => Color::DarkGray,
        1 => Color::LightRed,
        2 => Color::LightGreen,
        3 => Color::LightYellow,
        4 => Color::LightBlue,
        5 => Color::LightMagenta,
        6 => Color::LightCyan,
        7 => Color::White,
        _ => Color::Reset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::style::Stylize;

    fn span_text<'a>(line: &'a Line<'a>, index: usize) -> &'a str {
        line.spans[index].content.as_ref()
    }

    fn span_fg(line: &Line<'_>, index: usize) -> Option<Color> {
        line.spans[index].style.fg
    }

    #[test]
    fn empty_input_produces_an_empty_line() {
        let line = ansi_to_line("");
        assert!(line.spans.is_empty());
    }

    #[test]
    fn plain_text_becomes_one_span_with_default_style() {
        let line = ansi_to_line("hello world");
        assert_eq!(line.spans.len(), 1);
        assert_eq!(span_text(&line, 0), "hello world");
        assert_eq!(line.spans[0].style, Style::default());
    }

    #[test]
    fn rgb_foreground_is_preserved() {
        // Keep this as a literal SGR sequence instead of relying on
        // crossterm's Display impl. The Display path intentionally honors
        // environment such as NO_COLOR, but the parser must remain testable
        // in release shells that disable color globally.
        let raw = "\x1b[38;2;107;182;255m@backend\x1b[39m";
        let line = ansi_to_line(raw);
        assert!(line.spans.iter().any(|span| span.content == "@backend"
            && span.style.fg == Some(Color::Rgb(0x6b, 0xb6, 0xff))));
    }

    #[test]
    fn bold_modifier_is_preserved() {
        let raw = "ok".bold().to_string();
        let line = ansi_to_line(&raw);
        let bold = line
            .spans
            .iter()
            .find(|span| span.content == "ok")
            .expect("bold span");
        assert!(bold.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn reset_returns_to_default_style() {
        let raw = "\x1b[38;2;255;0;0mred\x1b[39mplain";
        let line = ansi_to_line(raw);
        // The "plain" span should not inherit the red fg.
        let plain = line
            .spans
            .iter()
            .find(|span| span.content == "plain")
            .expect("plain span");
        assert_eq!(plain.style.fg, Some(Color::Reset));
    }

    #[test]
    fn carriage_returns_are_dropped() {
        let line = ansi_to_line("hello\r");
        assert_eq!(span_text(&line, 0), "hello");
    }

    #[test]
    fn unknown_escapes_are_silently_skipped() {
        // Cursor move sequence in the middle of text.
        let raw = "before\x1b[2Aafter";
        let line = ansi_to_line(raw);
        let joined: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "beforeafter");
    }

    #[test]
    fn ansi_to_lines_splits_on_newlines() {
        let raw = "\x1b[38;2;50;50;50mone\x1b[39m\n\x1b[38;2;200;200;200mtwo\x1b[39m";
        let lines = ansi_to_lines(raw);
        assert_eq!(lines.len(), 2);
        assert_eq!(span_fg(&lines[0], 0), Some(Color::Rgb(50, 50, 50)));
        assert_eq!(span_fg(&lines[1], 0), Some(Color::Rgb(200, 200, 200)));
    }

    #[test]
    fn named_color_foreground_maps_to_ratatui_named() {
        // SGR 31 = red.
        let raw = "\x1b[31moops\x1b[0m";
        let line = ansi_to_line(raw);
        let oops = line
            .spans
            .iter()
            .find(|span| span.content == "oops")
            .expect("oops span");
        assert_eq!(oops.style.fg, Some(Color::Red));
    }

    #[test]
    fn indexed_256_color_foreground_is_preserved() {
        let raw = "\x1b[38;5;208mwarning\x1b[0m";
        let line = ansi_to_line(raw);
        let warning = line
            .spans
            .iter()
            .find(|span| span.content == "warning")
            .expect("warning span");
        assert_eq!(warning.style.fg, Some(Color::Indexed(208)));
    }

    #[test]
    fn italic_modifier_is_preserved() {
        let raw = "\x1b[3memph\x1b[23m";
        let line = ansi_to_line(raw);
        let emph = line
            .spans
            .iter()
            .find(|span| span.content == "emph")
            .expect("emph span");
        assert!(emph.style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn bold_then_reset_drops_bold() {
        let raw = "\x1b[1mbold\x1b[22mnot bold";
        let line = ansi_to_line(raw);
        let plain = line
            .spans
            .iter()
            .find(|span| span.content == "not bold")
            .expect("plain span");
        assert!(!plain.style.add_modifier.contains(Modifier::BOLD));
    }
}
