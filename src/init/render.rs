use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

use crossterm::style::Stylize;
use crossterm::terminal;

use crate::adapter::Engine;
use crate::config::CODEROOM_DIR;
use crate::detect;
use crate::output;

use super::{
    engine_color, engine_install_hint, engine_label, engine_note, human_label, model_label,
    project_name, role_color, role_info, InstalledEngines, RoleChoice, RoleInfo, RolePlan,
    DEFAULT_ENGINE,
};

/// Visible-cell budget used by the role picker layout.
///
/// `    [x] ● @<name>....description...`
///  ^^^^^^^^^^^^^^^^^^^
///  4    4   2  NAME_VISIBLE
///
/// The description fills whatever's left of the terminal width,
/// truncated (not wrapped) so each row stays exactly one line.
const NAME_VISIBLE: usize = 12;
const PICKER_PREFIX_VISIBLE: usize = 4 + 4 + 2 + NAME_VISIBLE;
const PICKER_RIGHT_MARGIN: usize = 2;
const PICKER_DEFAULT_COLS: u16 = 80;
const PICKER_MIN_DESC: usize = 16;

/// Detect terminal columns; fall back to 80 when unavailable
/// (non-TTY / piped output).
fn picker_columns() -> usize {
    terminal::size()
        .map_or(PICKER_DEFAULT_COLS, |(cols, _)| cols)
        .max(40) as usize
}

/// Render one row of the picker.
///
/// Each row is exactly one terminal line: prefix is fixed-width, the
/// description gets truncated with `…` so it can never wrap. Styling is
/// applied **after** all visible-width math so SGR escapes never leak
/// into padding budgets.
pub(super) fn picker_row(
    info: &RoleInfo,
    selected: bool,
    is_cursor: bool,
    columns: usize,
    extra_tag: Option<&str>,
) -> String {
    let cursor_glyph = if is_cursor { "  > " } else { "    " };
    let check = if selected { "[x] " } else { "[ ] " };

    // `@<name>` left-padded to NAME_VISIBLE visible cells. Pad PLAIN,
    // colour after — that's the bug the previous picker tripped on.
    let name_plain = format!("@{:<width$}", info.name, width = NAME_VISIBLE - 1);

    let mut desc_plain = info.description.to_owned();
    if let Some(tag) = extra_tag {
        desc_plain.push_str(" · ");
        desc_plain.push_str(tag);
    }
    let desc_budget = columns
        .saturating_sub(PICKER_PREFIX_VISIBLE)
        .saturating_sub(PICKER_RIGHT_MARGIN)
        .max(PICKER_MIN_DESC);
    let desc_truncated = output::truncate_visible(&desc_plain, desc_budget);

    let paint = role_color(info.name);
    format!(
        "{}{}{} {} {}",
        cursor_glyph.with(output::PROMPT),
        check.with(if is_cursor { output::EM } else { output::TEXT }),
        "●".with(paint),
        name_plain.with(paint).bold(),
        desc_truncated.with(output::DIM),
    )
}

pub(super) fn render_role_picker(
    project_root: &Path,
    scan: &detect::ProjectScan,
    choices: &[RoleChoice],
    cursor: usize,
) -> String {
    let project_name = project_name(project_root);
    let selected_count = choices.iter().filter(|choice| choice.selected).count();
    let columns = picker_columns();
    let mut out = String::new();

    push_header(
        &mut out,
        &project_name,
        "pick roles",
        "space toggles · ↑↓ moves · enter continues · esc backs out",
    );
    push_scan_compact(&mut out, scan);
    let _ = writeln!(out);

    for (index, choice) in choices.iter().enumerate() {
        let extra_tag = if choice.info.name == "host" {
            Some("required")
        } else {
            None
        };
        let _ = writeln!(
            out,
            "{}",
            picker_row(
                &choice.info,
                choice.selected,
                index == cursor,
                columns,
                extra_tag,
            )
        );
    }

    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "{}",
        format!("{selected_count} selected · host is always present · enter continues")
            .with(output::DIM)
    );
    out
}

pub(super) fn render_role_expansion_picker(
    project_root: &Path,
    scan: &detect::ProjectScan,
    choices: &[RoleChoice],
    cursor: usize,
) -> String {
    let project_name = project_name(project_root);
    let selected_count = choices
        .iter()
        .filter(|choice| choice.selected && choice.info.name != "host")
        .count();
    let columns = picker_columns();
    let mut out = String::new();

    push_header(
        &mut out,
        &project_name,
        "suggest roles",
        "space toggles · ↑↓ moves · enter adds selected · esc skips",
    );
    push_scan_compact(&mut out, scan);
    let _ = writeln!(
        out,
        "{}",
        "CodeRoom found only @host. Choose the specialists to add:".with(output::DIM)
    );
    let _ = writeln!(out);

    for (index, choice) in choices.iter().enumerate() {
        let extra_tag = if choice.info.name == "host" {
            Some("existing")
        } else {
            None
        };
        let _ = writeln!(
            out,
            "{}",
            picker_row(
                &choice.info,
                choice.selected,
                index == cursor,
                columns,
                extra_tag,
            )
        );
    }

    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "{}",
        format!("{selected_count} new role(s) selected · enter writes config and priors")
            .with(output::DIM)
    );
    out
}

pub(super) fn render_engine_picker(
    project_root: &Path,
    installed: &InstalledEngines,
    roles: &[String],
    assignments: &HashMap<String, Engine>,
    cursor: usize,
) -> String {
    // Pad PLAIN, style after — same fix as the role picker (`StyledContent`
    // includes SGR escapes when formatted, so `{:<N}` padding leaks into
    // the visible row width and the layout bleeds across lines).
    const ROLE_W: usize = 13;
    const ENGINE_W: usize = 10;
    const MODEL_W: usize = 18;

    let project_name = project_name(project_root);
    let mut out = String::new();

    push_header(
        &mut out,
        &project_name,
        "assign engines",
        "↑/↓ moves · ←/→ cycles engine · enter continues · esc goes back",
    );
    push_engine_status_compact(&mut out, installed);
    let _ = writeln!(out);
    let header = format!(
        "  {:<ROLE_W$} ‹ {:<ENGINE_W$} › {:<MODEL_W$} {}",
        "role", "engine", "model", "note"
    );
    let _ = writeln!(out, "{}", header.with(output::DIM));

    for (index, role) in roles.iter().enumerate() {
        let engine = *assignments.get(role).unwrap_or(&DEFAULT_ENGINE);
        let note = engine_note(engine, installed);
        let cursor_glyph = if index == cursor { "  > " } else { "    " };
        let role_plain = format!("@{role:<width$}", width = ROLE_W - 1);
        let engine_plain = format!("{:<ENGINE_W$}", engine_label(engine));
        let model_plain = format!("{:<MODEL_W$}", model_label(engine));
        let _ = writeln!(
            out,
            "{}{} ‹ {} › {} {}",
            cursor_glyph.with(output::PROMPT),
            role_plain.with(role_color(role)).bold(),
            engine_plain.with(engine_color(engine)),
            model_plain.with(output::DIM),
            note.with(output::DIM),
        );
    }

    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "{}",
        "defaults are editable later in .coderoom/config.toml".dark_grey()
    );
    out
}

pub(super) fn render_confirm(
    project_root: &Path,
    scan: &detect::ProjectScan,
    plan: &[RolePlan],
) -> String {
    let project_name = project_name(project_root);
    let coderoom_dir = project_root.join(CODEROOM_DIR);
    let mut out = String::new();

    push_header(
        &mut out,
        &project_name,
        "ready to write",
        "nothing is written until Enter",
    );

    let _ = writeln!(out, "will create:");
    let _ = writeln!(out);
    push_tree_preview(&mut out, &coderoom_dir, plan);
    let _ = writeln!(out);
    print_role_plan_to_buffer(&mut out, plan);

    if let Some(line_count) = scan.existing_claude_md() {
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "{} found existing {} ({} lines).",
            "!".yellow(),
            "CLAUDE.md".bold(),
            line_count
        );
        let _ = writeln!(
            out,
            "  {}",
            "coderoom will not touch it; split assistance can land separately.".dark_grey()
        );
    }

    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "{}",
        "enter writes · esc goes back · q aborts".dark_grey()
    );
    out
}

fn push_header(out: &mut String, project_name: &str, title: &str, subtitle: &str) {
    let _ = writeln!(
        out,
        "{} {} {}",
        title.bold(),
        "·".dark_grey(),
        format!("setting up coderoom in {project_name}").dark_grey()
    );
    let _ = writeln!(out, "{}", subtitle.dark_grey());
    let _ = writeln!(out);
}

fn push_scan_compact(out: &mut String, scan: &detect::ProjectScan) {
    if scan.stack.is_empty() {
        let _ = writeln!(
            out,
            "{}",
            "detected: no stack signals at project root".dark_grey()
        );
        return;
    }
    let labels = scan
        .stack
        .iter()
        .take(4)
        .map(human_label)
        .collect::<Vec<_>>()
        .join(" · ");
    let suffix = if scan.stack.len() > 4 { " · …" } else { "" };
    let _ = writeln!(
        out,
        "{} {}{}",
        "detected:".dark_grey(),
        labels,
        suffix.dark_grey()
    );
}

fn push_engine_status_compact(out: &mut String, installed: &InstalledEngines) {
    let _ = writeln!(out, "detected on your system:");
    for engine in [Engine::Cc, Engine::Codex, Engine::Gemini] {
        let label_padded = format!("{:<13}", engine_label(engine));
        if installed.is_present(engine) {
            let _ = writeln!(
                out,
                "  {} {} {}",
                "✓".with(output::OK),
                label_padded.with(engine_color(engine)),
                "installed".with(output::DIM),
            );
        } else {
            let _ = writeln!(
                out,
                "  {} {} {} {}",
                "✗".with(output::BAD),
                label_padded.with(output::DIM),
                "not installed ·".with(output::DIM),
                engine_install_hint(engine).with(output::WARN),
            );
        }
    }
}

fn push_tree_preview(out: &mut String, coderoom_dir: &Path, plan: &[RolePlan]) {
    let dirname = coderoom_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(CODEROOM_DIR);
    let _ = writeln!(out, "{dirname}/");
    let _ = writeln!(
        out,
        "├─ config.toml              {}",
        format!("{} roles", plan.len()).with(output::DIM)
    );
    let _ = writeln!(
        out,
        "├─ shared.md                {}",
        "project-wide priors".with(output::DIM)
    );
    let _ = writeln!(out, "├─ roles/");
    for (index, role) in plan.iter().enumerate() {
        let branch = if index + 1 == plan.len() {
            "└─"
        } else {
            "├─"
        };
        let role_filename = format!("{}.md", role.name);
        let role_filename_padded = format!("{role_filename:<18}");
        let _ = writeln!(
            out,
            "│  {branch} {} {}",
            role_filename_padded.with(role_color(&role.name)),
            engine_label(role.engine).with(output::DIM),
        );
    }
    let _ = writeln!(out, "└─ .gitignore");
}

fn print_role_plan_to_buffer(out: &mut String, plan: &[RolePlan]) {
    let header = format!("  {:<14} {:<12} {}", "role", "engine", "focus");
    let _ = writeln!(out, "{}", header.with(output::DIM));
    for role in plan {
        let info = role_info(&role.name);
        let role_token = format!("@{:<width$}", role.name, width = 13);
        let engine_padded = format!("{:<12}", engine_label(role.engine));
        let _ = writeln!(
            out,
            "  {} {} {}",
            role_token.with(role_color(&role.name)),
            engine_padded.with(engine_color(role.engine)),
            info.description.with(output::DIM),
        );
    }
}
