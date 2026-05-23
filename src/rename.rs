//! CoreRoom product identity constants.
//!
//! The repository, Rust crate, npm package, default project state directory,
//! and environment-variable prefix are all fully renamed in v0.7.0. The `cr`
//! binary stays stable as the short command.

use std::path::{Path, PathBuf};

/// Accepted product name.
pub const PRODUCT_NAME: &str = "CoreRoom";

/// Searchable descriptor used by active docs, metadata, and release notes.
pub const PRODUCT_DESCRIPTOR: &str = "Engineering Control Room for AI Agents";

/// Stable short command. It maps cleanly to CoreRoom and remains the happy
/// path after the rename.
pub const PRIMARY_COMMAND: &str = "cr";

/// Long command/package spelling where packaging supports it.
pub const LONG_COMMAND: &str = "coreroom";

/// Project state directory created by `cr init`.
pub const STATE_DIR: &str = ".coreroom";

/// npm package name.
pub const NPM_PACKAGE: &str = "@spytensor/coreroom";

/// Environment-variable prefix.
pub const ENV_PREFIX: &str = "COREROOM";

/// Resolution result for the local state directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateDirResolution {
    /// Directory CoreRoom should use.
    pub selected: PathBuf,
    /// Whether the directory already exists.
    pub exists: bool,
}

/// Resolve the current CoreRoom state directory without mutating the filesystem.
#[must_use]
pub fn resolve_state_dir(project_root: impl AsRef<Path>) -> StateDirResolution {
    let selected = project_root.as_ref().join(STATE_DIR);
    let exists = selected.is_dir();
    StateDirResolution { selected, exists }
}

/// Build a CoreRoom environment variable name for a suffix such as
/// `NO_UPDATE_CHECK`.
#[must_use]
pub fn env_name(suffix: &str) -> String {
    format!("{ENV_PREFIX}_{suffix}")
}
