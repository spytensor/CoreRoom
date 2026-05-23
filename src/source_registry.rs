//! Project-level source registry for host-led dependency context.
//!
//! The registry lists repos, docs, specs, policies, URL snapshots, and design
//! references that `@host` may consider when building future ContextPacks.
//! It is distinct from role knowledge: sources are project facts; role
//! knowledge is long-lived role-specific priors material.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::CODEROOM_DIR;

/// File inside `.coderoom/` that stores the project source registry.
pub const SOURCE_REGISTRY_FILE: &str = "source-registry.toml";

/// Current persisted Source Registry schema version.
pub const SOURCE_REGISTRY_SCHEMA_VERSION: u32 = 1;

/// Project source category.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum SourceKind {
    /// File from the current project checkout.
    ProjectFile,
    /// Local checkout of another repository.
    LocalRepo,
    /// Remote Git repository reference.
    GitRepo,
    /// Pinned snapshot of a URL.
    UrlSnapshot,
    /// Policy or compliance document.
    PolicyDoc,
    /// API specification such as OpenAPI, AsyncAPI, protobuf, or GraphQL SDL.
    ApiSpec,
    /// Design reference such as a design-system spec or exported screen doc.
    DesignReference,
}

impl SourceKind {
    /// Stable label used in persisted files and host output.
    pub const fn label(self) -> &'static str {
        match self {
            Self::ProjectFile => "project-file",
            Self::LocalRepo => "local-repo",
            Self::GitRepo => "git-repo",
            Self::UrlSnapshot => "url-snapshot",
            Self::PolicyDoc => "policy-doc",
            Self::ApiSpec => "api-spec",
            Self::DesignReference => "design-reference",
        }
    }

    const fn requires_path(self) -> bool {
        matches!(
            self,
            Self::ProjectFile
                | Self::LocalRepo
                | Self::PolicyDoc
                | Self::ApiSpec
                | Self::DesignReference
        )
    }

    const fn requires_url(self) -> bool {
        matches!(self, Self::GitRepo | Self::UrlSnapshot)
    }

    const fn requires_directory(self) -> bool {
        matches!(self, Self::LocalRepo)
    }
}

/// Trust level assigned by `@host` and the user.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum SourceTrustLevel {
    /// Current project source.
    Project,
    /// Internal team or organization source.
    Internal,
    /// External documentation.
    ExternalDoc,
    /// Policy or compliance authority.
    Policy,
    /// Generated artifact that needs separate verification.
    Generated,
    /// Explicitly untrusted reference material.
    Untrusted,
}

impl SourceTrustLevel {
    /// Stable label used in persisted files and host output.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Internal => "internal",
            Self::ExternalDoc => "external-doc",
            Self::Policy => "policy",
            Self::Generated => "generated",
            Self::Untrusted => "untrusted",
        }
    }
}

/// Refresh behavior for a pinned source.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "kebab-case")]
pub enum RefreshPolicy {
    /// Never refresh unless the source entry is replaced by the user.
    Never,
    /// Refresh only through a visible manual action.
    #[default]
    Manual,
    /// Host may propose refresh, but must ask confirmation before changing pins.
    OnConfirmation,
}

impl RefreshPolicy {
    /// Stable label used in persisted files and host output.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Never => "never",
            Self::Manual => "manual",
            Self::OnConfirmation => "on-confirmation",
        }
    }
}

/// One project dependency/context source.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSource {
    /// Stable source id.
    pub id: String,
    /// Source category.
    pub kind: SourceKind,
    /// Local path, relative to project root or absolute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Remote URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Pinned identity, such as `sha256:<hex>`, `commit:<sha>`, or `snapshot:<sha>`.
    pub pin: String,
    /// Trust level.
    pub trust_level: SourceTrustLevel,
    /// Human/team owner for freshness and correctness questions.
    pub owner: String,
    /// Roles allowed to see this source by default.
    pub visible_roles: Vec<String>,
    /// Why this source exists in the project registry.
    pub purpose: String,
    /// Refresh behavior. No policy permits silent remote refresh.
    #[serde(default)]
    pub refresh_policy: RefreshPolicy,
}

impl ProjectSource {
    /// Validate this source against local filesystem constraints.
    pub fn validate(&self, project_root: &Path) -> Result<()> {
        validate_source_id(&self.id)?;
        ensure_nonempty("pin", &self.pin)?;
        ensure_nonempty("owner", &self.owner)?;
        ensure_nonempty("purpose", &self.purpose)?;
        validate_visible_roles(&self.visible_roles)?;
        self.validate_location(project_root)?;
        Ok(())
    }

    fn validate_location(&self, project_root: &Path) -> Result<()> {
        match (self.kind.requires_path(), self.kind.requires_url()) {
            (true, false) => {
                let path = self.path.as_deref().ok_or_else(|| {
                    anyhow::anyhow!("{} source `{}` requires path", self.kind.label(), self.id)
                })?;
                if self.url.is_some() {
                    bail!(
                        "{} source `{}` must not set url",
                        self.kind.label(),
                        self.id
                    );
                }
                let resolved = resolve_source_path(project_root, path);
                if self.kind.requires_directory() {
                    if !resolved.is_dir() {
                        bail!(
                            "{} source `{}` path is not an accessible directory: {}",
                            self.kind.label(),
                            self.id,
                            resolved.display()
                        );
                    }
                    ensure_git_checkout(&resolved)
                } else if !resolved.is_file() {
                    bail!(
                        "{} source `{}` path is not an accessible file: {}",
                        self.kind.label(),
                        self.id,
                        resolved.display()
                    );
                } else {
                    Ok(())
                }
            }
            (false, true) => {
                let url = self.url.as_deref().ok_or_else(|| {
                    anyhow::anyhow!("{} source `{}` requires url", self.kind.label(), self.id)
                })?;
                if self.path.is_some() {
                    bail!(
                        "{} source `{}` must not set path",
                        self.kind.label(),
                        self.id
                    );
                }
                ensure_nonempty("url", url)?;
                if self.kind == SourceKind::UrlSnapshot && !is_http_url(url) {
                    bail!("url-snapshot source `{}` requires http(s) url", self.id);
                }
                Ok(())
            }
            _ => bail!("invalid source kind location contract"),
        }
    }
}

/// Persisted project source registry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceRegistry {
    /// Schema version.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Registered sources.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<ProjectSource>,
}

impl SourceRegistry {
    /// Create an empty registry.
    pub const fn new() -> Self {
        Self {
            schema_version: SOURCE_REGISTRY_SCHEMA_VERSION,
            sources: Vec::new(),
        }
    }

    /// Validate all sources and ids.
    pub fn validate(&self, project_root: &Path) -> Result<()> {
        if self.schema_version != SOURCE_REGISTRY_SCHEMA_VERSION {
            bail!(
                "unsupported Source Registry schemaVersion {}; expected {}",
                self.schema_version,
                SOURCE_REGISTRY_SCHEMA_VERSION
            );
        }
        let mut ids = BTreeSet::new();
        for source in &self.sources {
            source.validate(project_root)?;
            if !ids.insert(source.id.as_str()) {
                bail!("duplicate source id `{}`", source.id);
            }
        }
        Ok(())
    }

    /// Register a source after explicit confirmation.
    pub fn register_confirmed_source(
        &mut self,
        confirmed: ConfirmedSourceRegistration,
        project_root: &Path,
    ) -> Result<()> {
        confirmed.plan.source.validate(project_root)?;
        if self
            .sources
            .iter()
            .any(|source| source.id == confirmed.plan.source.id)
        {
            bail!(
                "source id `{}` is already registered",
                confirmed.plan.source.id
            );
        }
        self.sources.push(confirmed.plan.source);
        self.validate(project_root)
    }
}

impl Default for SourceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Confirmation-required source registration plan proposed by `@host`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceRegistrationPlan {
    /// Source proposed for registration.
    pub source: ProjectSource,
    /// Host must ask the user before registering this source.
    pub requires_confirmation: bool,
    /// Remote or external refresh is never silent in v0.6.
    pub allows_silent_refresh: bool,
}

impl SourceRegistrationPlan {
    /// Build a confirmation-required source registration plan.
    pub const fn new(source: ProjectSource) -> Self {
        Self {
            source,
            requires_confirmation: true,
            allows_silent_refresh: false,
        }
    }

    /// Convert this plan into a confirmed registration.
    pub fn confirm(self, confirmed_by: impl Into<String>) -> Result<ConfirmedSourceRegistration> {
        let confirmed_by = confirmed_by.into();
        ensure_nonempty("confirmedBy", &confirmed_by)?;
        Ok(ConfirmedSourceRegistration {
            plan: self,
            confirmed_by,
        })
    }
}

/// Confirmed source registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmedSourceRegistration {
    /// Original registration plan.
    pub plan: SourceRegistrationPlan,
    /// Actor who confirmed the registration.
    pub confirmed_by: String,
}

/// Save a source registry to `.coderoom/source-registry.toml`.
pub fn save_source_registry(project_root: &Path, registry: &SourceRegistry) -> Result<PathBuf> {
    registry.validate(project_root)?;
    let path = source_registry_path(project_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let content = toml::to_string_pretty(registry).context("serializing Source Registry")?;
    std::fs::write(&path, ensure_trailing_newline(&content))
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

/// Load and validate a source registry from a TOML file.
pub fn load_source_registry(path: &Path, project_root: &Path) -> Result<SourceRegistry> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let registry: SourceRegistry =
        toml::from_str(&content).with_context(|| format!("parsing {}", path.display()))?;
    registry.validate(project_root)?;
    Ok(registry)
}

/// Return the canonical source registry path for a project root.
pub fn source_registry_path(project_root: &Path) -> PathBuf {
    project_root.join(CODEROOM_DIR).join(SOURCE_REGISTRY_FILE)
}

fn default_schema_version() -> u32 {
    SOURCE_REGISTRY_SCHEMA_VERSION
}

fn validate_source_id(id: &str) -> Result<()> {
    ensure_nonempty("id", id)?;
    if id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        Ok(())
    } else {
        bail!("source id `{id}` may only use ASCII letters, digits, '-', '_', or '.'")
    }
}

fn validate_visible_roles(roles: &[String]) -> Result<()> {
    if roles.is_empty() {
        bail!("visibleRoles cannot be empty");
    }
    for role in roles {
        ensure_nonempty("visibleRoles entry", role)?;
        if role.contains('/') || role.contains('\\') {
            bail!("visibleRoles entry `{role}` must not contain path separators");
        }
    }
    Ok(())
}

fn ensure_nonempty(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{field} cannot be empty");
    }
    Ok(())
}

fn resolve_source_path(project_root: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_root.join(path)
    }
}

fn ensure_git_checkout(path: &Path) -> Result<()> {
    if path.join(".git").exists() {
        Ok(())
    } else {
        bail!(
            "local-repo source is not a Git checkout: {}",
            path.display()
        )
    }
}

fn is_http_url(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

fn ensure_trailing_newline(input: &str) -> String {
    if input.ends_with('\n') {
        input.to_owned()
    } else {
        format!("{input}\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_file_source(path: &str) -> ProjectSource {
        ProjectSource {
            id: "architecture-doc".to_owned(),
            kind: SourceKind::ProjectFile,
            path: Some(path.to_owned()),
            url: None,
            pin: "sha256:abc123".to_owned(),
            trust_level: SourceTrustLevel::Project,
            owner: "engineering".to_owned(),
            visible_roles: vec!["host".to_owned(), "engineer".to_owned()],
            purpose: "Architecture context for host planning.".to_owned(),
            refresh_policy: RefreshPolicy::Manual,
        }
    }

    #[test]
    fn registration_plan_requires_confirmation_and_no_silent_refresh() {
        let source = project_file_source("docs/architecture.md");
        let plan = SourceRegistrationPlan::new(source);

        assert!(plan.requires_confirmation);
        assert!(!plan.allows_silent_refresh);

        let confirmed = plan.confirm("user").expect("confirmed");
        assert_eq!(confirmed.confirmed_by, "user");
    }

    #[test]
    fn invalid_trust_level_fails_deserialization() {
        let content = r#"
schemaVersion = 1

[[sources]]
id = "bad-trust"
kind = "project-file"
path = "README.md"
pin = "sha256:abc123"
trustLevel = "magic"
owner = "engineering"
visibleRoles = ["host"]
purpose = "Prove canonical trust parsing."
refreshPolicy = "manual"
"#;

        let err = toml::from_str::<SourceRegistry>(content).expect_err("invalid trust");
        assert!(err.to_string().contains("unknown variant"));
    }
}
