use std::path::Path;

use crossterm::style::Color;

use crate::adapter::Engine;
use crate::detect::StackSignal;
use crate::output;

use super::{RoleInfo, ROLE_CATALOG, WIZARD_HOST_ROLE};

/// Human-readable bullet for a [`StackSignal`], used in the scan
/// summary. Kept here (not on the type) so the wording can evolve
/// independently of the detector's internals.
pub(super) fn human_label(signal: &StackSignal) -> String {
    match signal {
        StackSignal::CargoToml => "Cargo.toml (Rust)".into(),
        StackSignal::GoMod => "go.mod (Go)".into(),
        StackSignal::PackageJson {
            has_ui_framework: true,
        } => "package.json (with UI framework)".into(),
        StackSignal::PackageJson {
            has_ui_framework: false,
        } => "package.json (no UI framework detected)".into(),
        StackSignal::PythonProject => "Python project (requirements.txt or pyproject.toml)".into(),
        StackSignal::JvmProject => "JVM project (pom.xml or build.gradle)".into(),
        StackSignal::Migrations => "migrations/ or db/ directory".into(),
        StackSignal::Prisma => "prisma/ directory".into(),
        StackSignal::GithubWorkflows => ".github/workflows/".into(),
        StackSignal::Dockerfile => "Dockerfile".into(),
        StackSignal::Terraform => "terraform/".into(),
        StackSignal::Pulumi => "pulumi/".into(),
        StackSignal::Kubernetes => "k8s/ or kubernetes/".into(),
        StackSignal::ExistingClaudeMd { line_count } => format!("CLAUDE.md ({line_count} lines)"),
    }
}

/// Re-exported under the old name so the rest of `init.rs` doesn't have
/// to change. Single source of truth lives in `crate::engines`.
pub(super) type InstalledEngines = crate::engines::Engines;

pub(super) fn project_name(project_root: &Path) -> String {
    project_root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("(this project)")
        .to_owned()
}

pub(super) fn role_info(name: &str) -> RoleInfo {
    ROLE_CATALOG
        .iter()
        .copied()
        .find(|info| info.name == name)
        .unwrap_or(RoleInfo {
            name: "custom",
            description: "project-specific specialist",
        })
}

pub(super) fn role_color(role: &str) -> Color {
    output::role_color(role, WIZARD_HOST_ROLE)
}

pub(super) fn engine_color(engine: Engine) -> Color {
    match engine {
        Engine::Cc => Color::White,
        Engine::Codex => Color::Blue,
        Engine::Gemini => Color::Magenta,
        Engine::Fake => Color::DarkGrey,
    }
}

pub(super) fn engine_label(engine: Engine) -> &'static str {
    match engine {
        Engine::Cc => "claude-code",
        Engine::Codex => "codex",
        Engine::Gemini => "gemini-cli",
        Engine::Fake => "fake",
    }
}

pub(super) fn model_label(engine: Engine) -> &'static str {
    match engine {
        Engine::Cc => "claude default",
        Engine::Codex => "codex default",
        Engine::Gemini => "gemini default",
        Engine::Fake => "fake dogfood",
    }
}

pub(super) fn engine_install_hint(engine: Engine) -> &'static str {
    match engine {
        Engine::Cc => "docs.anthropic.com/claude-code",
        Engine::Codex => "github.com/openai/codex",
        Engine::Gemini => "github.com/google/gemini-cli",
        Engine::Fake => "set COREROOM_ENABLE_FAKE_ENGINE=1",
    }
}

pub(super) fn engine_note(engine: Engine, installed: &InstalledEngines) -> &'static str {
    if installed.is_present(engine) {
        "ready"
    } else {
        "install before cr start"
    }
}
