//! CoreRoom rename compatibility primitives.
//!
//! v0.7 accepts **CoreRoom** as the product name while keeping the shipped
//! `cr` command stable. This module centralizes the target spelling and the
//! compatibility policy for the two riskiest migration surfaces:
//! project state directories and environment-variable aliases.

use std::path::{Path, PathBuf};

/// Accepted product name.
pub const PRODUCT_NAME: &str = "CoreRoom";

/// Searchable descriptor used by active docs, metadata, and release notes.
pub const PRODUCT_DESCRIPTOR: &str = "Engineering Control Room for AI Agents";

/// Stable short command. It maps cleanly to CoreRoom and remains the happy
/// path during the rename.
pub const PRIMARY_COMMAND: &str = "cr";

/// Optional long command/package spelling where packaging supports it.
pub const LONG_COMMAND: &str = "coreroom";

/// Preferred future project state directory.
pub const CURRENT_STATE_DIR: &str = ".coreroom";

/// Legacy project state directory used by existing v0.x projects.
pub const LEGACY_STATE_DIR: &str = ".coderoom";

/// Preferred npm package after the package migration is published.
pub const CURRENT_NPM_PACKAGE: &str = "@spytensor/coreroom";

/// Legacy npm package kept for compatibility/deprecation.
pub const LEGACY_NPM_PACKAGE: &str = "@spytensor/coderoom";

/// Preferred environment-variable prefix after the rename.
pub const CURRENT_ENV_PREFIX: &str = "COREROOM";

/// Legacy environment-variable prefix supported during the compatibility
/// window.
pub const LEGACY_ENV_PREFIX: &str = "CODEROOM";

/// How a project root maps to CoreRoom state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateDirKind {
    /// `.coreroom/` exists and is the unambiguous state directory.
    Current,
    /// Only `.coderoom/` exists. It is usable as a legacy state directory,
    /// but migration should be explicit and visible to the user.
    Legacy,
    /// Neither state directory exists. New initialization should prefer
    /// `.coreroom/` once the write-path migration is enabled.
    MissingUseCurrent,
    /// Both directories exist. Automatic selection is unsafe.
    Conflict,
}

/// Resolution result for the local state directory migration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateDirResolution {
    /// Resolution category.
    pub kind: StateDirKind,
    /// Directory to use when the result is unambiguous.
    pub selected: Option<PathBuf>,
    /// Preferred current directory path.
    pub current: PathBuf,
    /// Legacy directory path.
    pub legacy: PathBuf,
    /// User-facing compatibility note.
    pub note: &'static str,
}

impl StateDirResolution {
    /// Whether runtime code may safely use `selected` without asking the
    /// user to resolve a conflict.
    #[must_use]
    pub const fn is_usable(&self) -> bool {
        !matches!(self.kind, StateDirKind::Conflict)
    }

    /// Whether the selected directory is a legacy `.coderoom/` directory.
    #[must_use]
    pub const fn uses_legacy(&self) -> bool {
        matches!(self.kind, StateDirKind::Legacy)
    }
}

/// Resolve state-directory compatibility without mutating the filesystem.
#[must_use]
pub fn resolve_state_dir(project_root: impl AsRef<Path>) -> StateDirResolution {
    let project_root = project_root.as_ref();
    let current = project_root.join(CURRENT_STATE_DIR);
    let legacy = project_root.join(LEGACY_STATE_DIR);
    let current_exists = current.is_dir();
    let legacy_exists = legacy.is_dir();

    match (current_exists, legacy_exists) {
        (true, false) => StateDirResolution {
            kind: StateDirKind::Current,
            selected: Some(current.clone()),
            current,
            legacy,
            note: "using CoreRoom state directory",
        },
        (false, true) => StateDirResolution {
            kind: StateDirKind::Legacy,
            selected: Some(legacy.clone()),
            current,
            legacy,
            note: "using legacy CodeRoom state directory; migration requires explicit confirmation",
        },
        (false, false) => StateDirResolution {
            kind: StateDirKind::MissingUseCurrent,
            selected: Some(current.clone()),
            current,
            legacy,
            note: "no state directory exists; new projects should initialize CoreRoom state",
        },
        (true, true) => StateDirResolution {
            kind: StateDirKind::Conflict,
            selected: None,
            current,
            legacy,
            note: "both CoreRoom and legacy CodeRoom state directories exist; user must resolve",
        },
    }
}

/// Which spelling supplied a resolved environment value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvAliasSource {
    /// The new `COREROOM_*` variable supplied the value.
    Current,
    /// The legacy `CODEROOM_*` variable supplied the value.
    Legacy,
    /// Both were set; the new spelling wins.
    CurrentPreferredOverLegacy,
    /// Neither spelling was set.
    Missing,
}

/// Environment-variable alias resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvAliasResolution<'a> {
    /// Preferred variable name.
    pub current_name: &'a str,
    /// Legacy variable name.
    pub legacy_name: &'a str,
    /// Resolved value, if any.
    pub value: Option<&'a str>,
    /// Source of the resolved value.
    pub source: EnvAliasSource,
    /// Whether a compatibility warning is appropriate.
    pub legacy_used: bool,
}

/// Resolve a `COREROOM_*` / `CODEROOM_*` alias pair from already-read values.
///
/// This is intentionally value-based so tests do not mutate global process
/// environment and so call sites can decide how loudly to warn.
#[must_use]
pub fn resolve_env_alias_from_values<'a>(
    current_name: &'a str,
    current_value: Option<&'a str>,
    legacy_name: &'a str,
    legacy_value: Option<&'a str>,
) -> EnvAliasResolution<'a> {
    match (current_value, legacy_value) {
        (Some(value), Some(_)) => EnvAliasResolution {
            current_name,
            legacy_name,
            value: Some(value),
            source: EnvAliasSource::CurrentPreferredOverLegacy,
            legacy_used: true,
        },
        (Some(value), None) => EnvAliasResolution {
            current_name,
            legacy_name,
            value: Some(value),
            source: EnvAliasSource::Current,
            legacy_used: false,
        },
        (None, Some(value)) => EnvAliasResolution {
            current_name,
            legacy_name,
            value: Some(value),
            source: EnvAliasSource::Legacy,
            legacy_used: true,
        },
        (None, None) => EnvAliasResolution {
            current_name,
            legacy_name,
            value: None,
            source: EnvAliasSource::Missing,
            legacy_used: false,
        },
    }
}

/// Build a preferred CoreRoom environment variable name for a suffix such as
/// `NO_UPDATE_CHECK`.
#[must_use]
pub fn current_env_name(suffix: &str) -> String {
    format!("{CURRENT_ENV_PREFIX}_{suffix}")
}

/// Build a legacy CodeRoom environment variable name for a suffix such as
/// `NO_UPDATE_CHECK`.
#[must_use]
pub fn legacy_env_name(suffix: &str) -> String {
    format!("{LEGACY_ENV_PREFIX}_{suffix}")
}
