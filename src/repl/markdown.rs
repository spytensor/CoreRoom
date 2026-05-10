use crossterm::style::Stylize;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::output;

const MIN_TEXT_WIDTH: usize = 8;

pub(super) fn render_role_markdown(
    role: &str,
    host_role: &str,
    text: &str,
    width: usize,
) -> String {
    let role_color = output::role_color(role, host_role);
    let first_prefix = format!(
        "{} {} ",
        super::render::GUTTER.with(role_color),
        output::role_token(role, host_role)
    );
    let rest_prefix = format!("{} ", super::render::GUTTER.with(role_color));
    let first_plain = format!("{} @{role} ", super::render::GUTTER);
    let rest_plain = format!("{} ", super::render::GUTTER);
    let mut renderer = Renderer {
        width,
        first_prefix,
        rest_prefix,
        first_prefix_width: UnicodeWidthStr::width(first_plain.as_str()),
        rest_prefix_width: UnicodeWidthStr::width(rest_plain.as_str()),
        first_line: true,
        lines: Vec::new(),
    };
    render_blocks(text, &mut renderer);
    renderer.lines.join("\n")
}

struct Renderer {
    width: usize,
    first_prefix: String,
    rest_prefix: String,
    first_prefix_width: usize,
    rest_prefix_width: usize,
    first_line: bool,
    lines: Vec<String>,
}

impl Renderer {
    fn available(&self) -> usize {
        let prefix = if self.first_line {
            self.first_prefix_width
        } else {
            self.rest_prefix_width
        };
        self.width.saturating_sub(prefix).max(MIN_TEXT_WIDTH)
    }

    fn push_blank(&mut self) {
        let prefix = if self.first_line {
            &self.first_prefix
        } else {
            &self.rest_prefix
        };
        self.lines.push(prefix.trim_end().to_owned());
        self.first_line = false;
    }

    fn push_wrapped(&mut self, text: &str, style: LineStyle, continuation_indent: &str) {
        let available = self.available();
        let wrapped = wrap_cells(text, available, continuation_indent);
        if wrapped.is_empty() {
            self.push_blank();
            return;
        }
        for line in wrapped {
            let prefix = if self.first_line {
                &self.first_prefix
            } else {
                &self.rest_prefix
            };
            self.lines
                .push(format!("{prefix}{}", apply_line_style(&line, style)));
            self.first_line = false;
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum LineStyle {
    Normal,
    Emphasis,
    Code,
}

fn apply_line_style(text: &str, style: LineStyle) -> String {
    match style {
        LineStyle::Normal => render_inline_bold(text),
        LineStyle::Emphasis => strip_bold_markers(text).with(output::EM).bold().to_string(),
        LineStyle::Code => text.with(output::DIM).to_string(),
    }
}

fn render_inline_bold(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    let mut bold = false;
    while let Some(idx) = rest.find("**") {
        let (head, tail) = rest.split_at(idx);
        if bold {
            out.push_str(&head.with(output::TEXT).bold().to_string());
        } else {
            out.push_str(&head.with(output::TEXT).to_string());
        }
        rest = &tail[2..];
        bold = !bold;
    }
    if bold {
        out.push_str(&rest.with(output::TEXT).bold().to_string());
    } else {
        out.push_str(&rest.with(output::TEXT).to_string());
    }
    out
}

fn strip_bold_markers(text: &str) -> String {
    text.replace("**", "")
}

fn render_blocks(text: &str, renderer: &mut Renderer) {
    let mut in_code = false;

    for raw in text.lines() {
        let line = raw.trim_end();
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            renderer.push_wrapped(line, LineStyle::Code, "");
            continue;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            renderer.push_blank();
            continue;
        }

        if let Some(heading) = heading(trimmed) {
            renderer.push_wrapped(&strip_bold_markers(heading), LineStyle::Emphasis, "");
            continue;
        }

        if let Some(item) = bullet(trimmed) {
            renderer.push_wrapped(&format!("• {item}"), LineStyle::Normal, "  ");
            continue;
        }

        renderer.push_wrapped(trimmed, LineStyle::Normal, "");
    }
}

fn heading(line: &str) -> Option<&str> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if (1..=3).contains(&hashes) && line.chars().nth(hashes) == Some(' ') {
        Some(line[hashes + 1..].trim())
    } else {
        None
    }
}

fn bullet(line: &str) -> Option<&str> {
    let marker = line.chars().next()?;
    if matches!(marker, '-' | '*' | '+') && line.chars().nth(1) == Some(' ') {
        Some(line[2..].trim())
    } else {
        None
    }
}

fn wrap_cells(text: &str, width: usize, continuation_indent: &str) -> Vec<String> {
    let width = width.max(MIN_TEXT_WIDTH);
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    for word in text.split_whitespace() {
        let word_width = UnicodeWidthStr::width(word);
        if word_width > width {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
                current.push_str(continuation_indent);
                current_width = UnicodeWidthStr::width(continuation_indent);
            }
            for chunk in hard_wrap_word(word, width) {
                if current_width + UnicodeWidthStr::width(chunk.as_str()) > width {
                    lines.push(std::mem::take(&mut current));
                    current.push_str(continuation_indent);
                    current_width = UnicodeWidthStr::width(continuation_indent);
                }
                current.push_str(&chunk);
                current_width += UnicodeWidthStr::width(chunk.as_str());
                if current_width >= width {
                    lines.push(std::mem::take(&mut current));
                    current.push_str(continuation_indent);
                    current_width = UnicodeWidthStr::width(continuation_indent);
                }
            }
            continue;
        }
        let separator = usize::from(!current.trim().is_empty());
        if current_width + separator + word_width > width {
            lines.push(std::mem::take(&mut current));
            current.push_str(continuation_indent);
            current_width = UnicodeWidthStr::width(continuation_indent);
        }
        if !current.trim().is_empty() {
            current.push(' ');
            current_width += 1;
        }
        current.push_str(word);
        current_width += word_width;
    }
    if !current.is_empty() || text.is_empty() {
        lines.push(current);
    }
    lines
}

fn hard_wrap_word(word: &str, width: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut used = 0usize;
    for ch in word.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + ch_width > width && !current.is_empty() {
            chunks.push(current);
            current = String::new();
            used = 0;
        }
        current.push(ch);
        used += ch_width;
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}
