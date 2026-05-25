//! Project-level configuration loaded from `.coreroom/config.toml`.
//!
//! A project's `.coreroom/` layout (per `docs/architecture.md`):
//!
//! ```text
//! .coreroom/
//! ├── config.toml             # this file
//! ├── roles/<name>/priors.md  # per-role base priors
//! ├── roles/<name>/knowledge/ # mounted domain documents
//! ├── shared.md               # priors loaded by every role (optional)
//! ├── patches/<role>/...      # session-time corrections
//! └── journal/YYYY-MM-DD/...  # daily learnings, role-written
//! ```
//!
//! `config.toml` shape:
//!
//! ```toml
//! default_engine = "cc"          # cc | codex | gemini | fake (dogfood only)
//! default_model = "opus"         # optional; engine-specific id
//! permission_mode = "ask"        # ask | auto | bypass
//! host_role = "pm"               # role that catches un-addressed text
//!
//! [roles.pm]
//! # engine inherits default_engine; model inherits default_model
//!
//! [roles.security]
//! engine = "codex"
//! model = "o3"
//! owner = "alice@example.com"
//! access = "read-review" # host-control | write | read-review
//! authority = ["deployment", "infra", "secrets"]
//! ```

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::adapter::{Engine, PermissionMode, RoleConfig};

/// Standard subdirectory inside a project that holds legacy CoreRoom state.
pub const COREROOM_DIR: &str = ".coreroom";

/// File name of the project-level config inside [`COREROOM_DIR`].
pub const CONFIG_FILE: &str = "config.toml";

/// Subdirectory holding per-role priors files.
pub const ROLES_DIR: &str = "roles";

/// Project-level config loaded from `.coreroom/config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    /// Engine used for any role that doesn't override.
    pub default_engine: Engine,
    /// Model id used for any role that doesn't override. Engine-specific
    /// (e.g. `"opus"` for `cc`, `"o3"` for `codex`).
    #[serde(default)]
    pub default_model: Option<String>,
    /// Default permission mode for any role that doesn't override.
    #[serde(default = "default_permission_mode")]
    pub permission_mode: PermissionMode,
    /// Name of the role that catches un-addressed text in the REPL.
    /// Must exist in [`Self::roles`].
    pub host_role: String,
    /// Per-role overrides. Each key is a role name; the table allows
    /// engine/model to differ from the defaults. Entries are optional —
    /// a role with no entry uses the defaults.
    #[serde(default)]
    pub roles: HashMap<String, RoleEntry>,
}

/// Per-role overrides in `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct RoleEntry {
    /// Engine override. `None` ⇒ use [`Config::default_engine`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<Engine>,
    /// Model override. `None` ⇒ use [`Config::default_model`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Permission mode override. `None` ⇒ use [`Config::permission_mode`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<PermissionMode>,
    /// Human owner responsible for this role's priors and authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Role access class. This is separate from tool permission mode:
    /// `permission_mode` gates tool approval, while `access` describes
    /// whether the role is host control, implementation/write, or
    /// read/review by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<RoleAccess>,
    /// Canonical scopes where this role may issue a binding plan veto.
    /// This is domain authority, not file-write permission.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authority: Vec<AuthorityScope>,
}

/// Default access class for a role before WorkOrder-scoped grants.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum RoleAccess {
    /// User-facing project control role. This does not make the user
    /// less authoritative; it means the role may coordinate CoreRoom
    /// project state on behalf of the user.
    HostControl,
    /// Scoped implementation role that may be assigned file/test work.
    Write,
    /// Read, review, tool-assisted analysis, and suggestions only.
    ReadReview,
}

impl RoleAccess {
    /// Canonical config value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HostControl => "host-control",
            Self::Write => "write",
            Self::ReadReview => "read-review",
        }
    }
}

impl fmt::Display for RoleAccess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Canonical authority scopes a role may be allowed to veto.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum AuthorityScope {
    /// Deployment and rollout mechanics.
    Deployment,
    /// Infrastructure topology, runtime platform, and operations standards.
    Infra,
    /// Secret handling, credential storage, and sensitive runtime config.
    Secrets,
    /// Data retention, classification, and movement policy.
    DataPolicy,
    /// Compliance or regulatory constraints.
    Compliance,
    /// Dependency selection, updates, and supply-chain policy.
    Dependencies,
}

impl AuthorityScope {
    /// All accepted scope values in canonical display order.
    pub const ALL: [Self; 6] = [
        Self::Deployment,
        Self::Infra,
        Self::Secrets,
        Self::DataPolicy,
        Self::Compliance,
        Self::Dependencies,
    ];

    /// Canonical kebab-case config value for this scope.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Deployment => "deployment",
            Self::Infra => "infra",
            Self::Secrets => "secrets",
            Self::DataPolicy => "data-policy",
            Self::Compliance => "compliance",
            Self::Dependencies => "dependencies",
        }
    }

    /// Parse a canonical config value.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|scope| scope.as_str() == value)
    }

    /// Comma-separated list of accepted config values for diagnostics.
    #[must_use]
    pub fn expected_values() -> String {
        Self::ALL
            .iter()
            .map(|scope| scope.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl fmt::Display for AuthorityScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

const fn default_permission_mode() -> PermissionMode {
    PermissionMode::Ask
}

/// Errors raised while loading or validating a config.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// `.coreroom/config.toml` is missing or unreadable.
    #[error("could not read {path}: {source}")]
    Read {
        /// Absolute path that failed to read.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// `.coreroom/config.toml` content is not valid TOML / shape.
    #[error("could not parse {path}: {source}")]
    Parse {
        /// Absolute path that failed to parse.
        path: PathBuf,
        /// Underlying TOML deserialization error.
        #[source]
        source: toml::de::Error,
    },
    /// `host_role` was not declared as a role.
    #[error("host_role `{host}` is not declared in [roles] (declared: {declared:?})")]
    MissingHostRole {
        /// Value of `host_role` that didn't resolve.
        host: String,
        /// Role names that *were* declared.
        declared: Vec<String>,
    },
    /// A role exists in config but its priors file is missing on disk.
    #[error("role `{role}` is missing priors file at {expected}")]
    MissingPriors {
        /// Role name that lacks priors.
        role: String,
        /// Path the loader checked.
        expected: PathBuf,
    },
    /// A categorical scoping rule was violated: a key only allowed in
    /// one specific layer was found in a different layer (e.g.
    /// `engines.cc.bin` in committed project config, or `[roles]` in
    /// user config).
    #[error("`{field}` is not allowed in {path}: {why}")]
    Forbidden {
        /// Path of the offending file.
        path: PathBuf,
        /// Field path (e.g. `engines.cc.bin`).
        field: String,
        /// Human-readable explanation pointing the user at the right
        /// layer.
        why: String,
    },
    /// No layer declared `default_engine`. Built-in fallback isn't
    /// applied because choosing an engine without consent is a real
    /// failure mode.
    #[error(
        "no default_engine is declared. set it in either user config \
         (~/.config/coreroom/config.toml under [defaults] engine = \"cc\") \
         or project config (.coreroom/config.toml `default_engine = \"cc\"`)."
    )]
    MissingDefaultEngine,
    /// The deterministic fake engine is present in config without its
    /// explicit dogfood/test gate.
    #[error(
        "engine `fake` is dogfood/test-only; set COREROOM_ENABLE_FAKE_ENGINE=1 to load roles: {roles:?}"
    )]
    FakeEngineDisabled {
        /// Roles that would run on the fake engine.
        roles: Vec<String>,
    },
}

/// Convenience alias for config results.
pub type ConfigResult<T> = Result<T, ConfigError>;

impl Config {
    /// Load and validate a project's `.coreroom/config.toml`.
    ///
    /// `project_root` is the directory containing `.coreroom/` — typically
    /// the user's project repo root. Validation includes:
    ///
    /// 1. TOML parses into the documented shape.
    /// 2. `host_role` is one of the declared roles.
    /// 3. Every declared role has a priors file at
    ///    `.coreroom/roles/<role>/priors.md`, with legacy
    ///    `.coreroom/roles/<role>.md` still accepted.
    pub fn load(project_root: impl AsRef<Path>) -> ConfigResult<Self> {
        // Delegate to the layered loader. Production code resolves
        // the user-config path from $XDG_CONFIG_HOME / ~/.config etc.;
        // tests can call `crate::config_layered::load` directly to
        // pass a hermetic user path (or `None`).
        crate::config_layered::load(
            project_root.as_ref(),
            crate::config_layered::user_config_path().as_deref(),
        )
    }

    /// Validate the in-memory config against on-disk state. Used by
    /// [`Self::load`]; exposed so tests can validate hand-built configs.
    pub fn validate(&self, coreroom_dir: &Path) -> ConfigResult<()> {
        if !self.roles.contains_key(&self.host_role) {
            let mut declared: Vec<String> = self.roles.keys().cloned().collect();
            declared.sort();
            return Err(ConfigError::MissingHostRole {
                host: self.host_role.clone(),
                declared,
            });
        }

        let mut fake_roles = self
            .roles
            .iter()
            .filter_map(|(name, entry)| {
                let engine = entry.engine.unwrap_or(self.default_engine);
                (engine == Engine::Fake).then_some(name.clone())
            })
            .collect::<Vec<_>>();
        fake_roles.sort();
        if !fake_roles.is_empty() && !crate::adapter::fake::enabled() {
            return Err(ConfigError::FakeEngineDisabled { roles: fake_roles });
        }

        for name in self.roles.keys() {
            let priors = priors_path_for(coreroom_dir, name);
            if !priors.is_file() {
                return Err(ConfigError::MissingPriors {
                    role: name.clone(),
                    expected: priors,
                });
            }
        }

        Ok(())
    }

    /// Whether the given role is the configured host.
    #[must_use]
    pub fn is_host(&self, role: &str) -> bool {
        self.host_role == role
    }

    /// Effective role access before any WorkOrder-scoped escalation.
    ///
    /// This intentionally does not inspect [`PermissionMode`]:
    /// permission mode is tool-supervision policy, while role access is
    /// the project/work authority class shown in status surfaces.
    #[must_use]
    pub fn effective_role_access(&self, role: &str) -> RoleAccess {
        if self.is_host(role) {
            return RoleAccess::HostControl;
        }
        self.roles
            .get(role)
            .and_then(|entry| entry.access)
            .unwrap_or_else(|| {
                if role == "engineer" {
                    RoleAccess::Write
                } else {
                    RoleAccess::ReadReview
                }
            })
    }

    /// Build an [`adapter::RoleConfig`](crate::adapter::RoleConfig) for
    /// the named role. Resolves engine/model from per-role overrides
    /// falling back to defaults.
    ///
    /// Returns `None` if the role is not declared in the config.
    #[must_use]
    pub fn role_config(&self, name: &str, coreroom_dir: &Path) -> Option<RoleConfig> {
        let entry = self.roles.get(name)?;
        let engine = entry.engine.unwrap_or(self.default_engine);
        let permission_mode = entry.permission_mode.unwrap_or(match engine {
            // Existing projects created before per-role permission modes may
            // have Codex/Gemini roles with no explicit override. Keep those
            // roles startable as bypass; explicit ask/auto settings are
            // still validated by each adapter's current capability surface.
            Engine::Codex | Engine::Gemini => PermissionMode::Bypass,
            Engine::Cc | Engine::Fake => self.permission_mode,
        });
        Some(RoleConfig {
            name: name.to_owned(),
            engine,
            model: entry.model.clone().or_else(|| self.default_model.clone()),
            priors_path: priors_path_for(coreroom_dir, name),
            permission_mode,
            permission_policy_path: None,
            permission_socket_path: None,
            // Populated by the REPL spawn path after reading
            // `.coreroom/sessions/ids/<role>.id`. The bare `Config`
            // loader is engine-neutral and doesn't see session state.
            resume_session_id: None,
        })
    }

    /// Iterator over declared role names, in arbitrary order.
    pub fn role_names(&self) -> impl Iterator<Item = &str> {
        self.roles.keys().map(String::as_str)
    }

    /// Test-only hermetic loader that skips the user layer entirely.
    /// Used by unit tests so they don't pick up the developer's
    /// real `~/.config/coreroom/config.toml` and become flaky on
    /// machines where the user has actually configured CoreRoom.
    #[cfg(test)]
    pub(crate) fn load_test(project_root: impl AsRef<Path>) -> ConfigResult<Self> {
        crate::config_layered::load(project_root.as_ref(), None)
    }
}

/// Path where the priors file for `role` lives, given the project's
/// `.coreroom/` directory. The directory layout is preferred, while
/// legacy flat `.md` files remain accepted for back compatibility.
fn priors_path_for(coreroom_dir: &Path, role: &str) -> PathBuf {
    crate::manifest::role_priors_path_for_config(coreroom_dir, role)
}

#[cfg(test)]
mod tests;
