//! WorkOrder-scoped ContextPack model.
//!
//! A ContextPack selects small, explicit slices from the project Source
//! Registry for a specific WorkOrder and target roles. It does not load every
//! source into every role by default.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::COREROOM_DIR;
use crate::source_registry::{ProjectSource, SourceKind, SourceRegistry, SourceTrustLevel};

/// Subdirectory inside `.coreroom/` that stores ContextPacks.
pub const CONTEXT_PACKS_DIR: &str = "context-packs";

/// Current persisted ContextPack schema version.
pub const CONTEXT_PACK_SCHEMA_VERSION: u32 = 1;

/// A selected line range inside a source path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextRange {
    /// First selected line, 1-based.
    pub start_line: u32,
    /// Last selected line, 1-based and inclusive.
    pub end_line: u32,
}

impl ContextRange {
    /// Validate range order and one-based line numbers.
    pub fn validate(&self) -> Result<()> {
        if self.start_line == 0 || self.end_line == 0 {
            bail!("context range lines are 1-based and must be greater than zero");
        }
        if self.start_line > self.end_line {
            bail!(
                "context range startLine {} is after endLine {}",
                self.start_line,
                self.end_line
            );
        }
        Ok(())
    }
}

/// One selected source slice for one or more target roles.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextPackEntry {
    /// Source Registry id.
    pub source_id: String,
    /// Selected local path or path inside a repo/spec.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Optional line range for path-based selections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<ContextRange>,
    /// Snapshot reference for URL or remote-source selections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_ref: Option<String>,
    /// Why this slice is needed.
    pub reason: String,
    /// Roles that should receive this slice.
    pub target_roles: Vec<String>,
    /// Source pin copied from the registry when the pack was built.
    pub source_pin: String,
    /// Trust level copied from the source registry for audit display.
    pub trust_level: SourceTrustLevel,
}

impl ContextPackEntry {
    /// Validate entry shape without consulting the Source Registry.
    pub fn validate_shape(&self) -> Result<()> {
        ensure_source_id("sourceId", &self.source_id)?;
        ensure_nonempty("reason", &self.reason)?;
        ensure_nonempty("sourcePin", &self.source_pin)?;
        validate_roles("targetRoles", &self.target_roles)?;
        if let Some(range) = &self.range {
            range.validate()?;
            if self.path.is_none() {
                bail!("range requires path for source `{}`", self.source_id);
            }
        }
        if self.path.is_none() && self.snapshot_ref.is_none() {
            bail!(
                "context entry for source `{}` requires path or snapshotRef",
                self.source_id
            );
        }
        if let Some(path) = &self.path {
            ensure_nonempty("path", path)?;
        }
        if let Some(snapshot_ref) = &self.snapshot_ref {
            ensure_nonempty("snapshotRef", snapshot_ref)?;
        }
        Ok(())
    }
}

/// Persisted ContextPack for one WorkOrder.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextPack {
    /// Schema version.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Stable ContextPack id, usually `CTX-<WorkOrder id>`.
    pub id: String,
    /// Bound WorkOrder id.
    pub work_order: String,
    /// Selected source slices.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<ContextPackEntry>,
}

impl ContextPack {
    /// Create an empty ContextPack for a WorkOrder.
    pub fn new(id: impl Into<String>, work_order: impl Into<String>) -> Self {
        Self {
            schema_version: CONTEXT_PACK_SCHEMA_VERSION,
            id: id.into(),
            work_order: work_order.into(),
            entries: Vec::new(),
        }
    }

    /// Validate this pack against a Source Registry.
    pub fn validate_against_registry(
        &self,
        registry: &SourceRegistry,
    ) -> Result<ContextPackValidation> {
        if self.schema_version != CONTEXT_PACK_SCHEMA_VERSION {
            bail!(
                "unsupported ContextPack schemaVersion {}; expected {}",
                self.schema_version,
                CONTEXT_PACK_SCHEMA_VERSION
            );
        }
        validate_context_pack_id(&self.id)?;
        validate_work_order_id(&self.work_order)?;
        if self.entries.is_empty() {
            bail!("ContextPack entries cannot be empty");
        }

        let mut warnings = Vec::new();
        for entry in &self.entries {
            entry.validate_shape()?;
            let source = source_by_id(registry, &entry.source_id)
                .ok_or_else(|| anyhow::anyhow!("source `{}` is not registered", entry.source_id))?;
            validate_entry_against_source(entry, source, &mut warnings)?;
        }
        Ok(ContextPackValidation { warnings })
    }
}

/// Validation report returned by ContextPack checks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContextPackValidation {
    /// Non-blocking warnings, such as stale source pins.
    pub warnings: Vec<String>,
}

/// Confirmation-aware ContextPack proposal from `@host`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextPackProposal {
    /// Proposed ContextPack.
    pub context_pack: ContextPack,
    /// Host should ask before persisting or delegating from this pack.
    pub requires_confirmation: bool,
}

impl ContextPackProposal {
    /// Build a confirmation-required ContextPack proposal.
    pub const fn new(context_pack: ContextPack) -> Self {
        Self {
            context_pack,
            requires_confirmation: true,
        }
    }

    /// Convert this proposal into a confirmed ContextPack.
    pub fn confirm(self, confirmed_by: impl Into<String>) -> Result<ConfirmedContextPack> {
        let confirmed_by = confirmed_by.into();
        ensure_nonempty("confirmedBy", &confirmed_by)?;
        Ok(ConfirmedContextPack {
            proposal: self,
            confirmed_by,
        })
    }
}

/// Confirmed ContextPack ready for persistence or delegation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmedContextPack {
    /// Original proposal.
    pub proposal: ContextPackProposal,
    /// Actor who confirmed it.
    pub confirmed_by: String,
}

/// Save a ContextPack under `.coreroom/context-packs/<id>.toml`.
pub fn save_context_pack(
    project_root: &Path,
    registry: &SourceRegistry,
    pack: &ContextPack,
) -> Result<PathBuf> {
    pack.validate_against_registry(registry)?;
    let path = context_pack_path(project_root, &pack.id)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let content = toml::to_string_pretty(pack).context("serializing ContextPack")?;
    std::fs::write(&path, ensure_trailing_newline(&content))
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

/// Load and validate a ContextPack TOML file.
pub fn load_context_pack(path: &Path, registry: &SourceRegistry) -> Result<ContextPack> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let pack: ContextPack =
        toml::from_str(&content).with_context(|| format!("parsing {}", path.display()))?;
    pack.validate_against_registry(registry)?;
    Ok(pack)
}

/// Return the canonical ContextPack path for a pack id.
pub fn context_pack_path(project_root: &Path, id: &str) -> Result<PathBuf> {
    validate_context_pack_id(id)?;
    Ok(project_root
        .join(COREROOM_DIR)
        .join(CONTEXT_PACKS_DIR)
        .join(format!("{id}.toml")))
}

fn default_schema_version() -> u32 {
    CONTEXT_PACK_SCHEMA_VERSION
}

fn source_by_id<'a>(registry: &'a SourceRegistry, source_id: &str) -> Option<&'a ProjectSource> {
    registry
        .sources
        .iter()
        .find(|source| source.id == source_id)
}

fn validate_entry_against_source(
    entry: &ContextPackEntry,
    source: &ProjectSource,
    warnings: &mut Vec<String>,
) -> Result<()> {
    if source.pin.trim().is_empty() {
        warnings.push(format!("source `{}` is unpinned", source.id));
    } else if source.pin != entry.source_pin {
        warnings.push(format!(
            "source `{}` pin is stale: ContextPack has `{}`, registry has `{}`",
            source.id, entry.source_pin, source.pin
        ));
    }
    if source.trust_level != entry.trust_level {
        warnings.push(format!(
            "source `{}` trust level changed: ContextPack has `{}`, registry has `{}`",
            source.id,
            entry.trust_level.label(),
            source.trust_level.label()
        ));
    }

    let visible_roles = source
        .visible_roles
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    for role in &entry.target_roles {
        if !visible_roles.contains(role.as_str()) {
            bail!(
                "role `{role}` is not visible for source `{}`; allowed roles: {}",
                source.id,
                source.visible_roles.join(", ")
            );
        }
    }

    match source.kind {
        SourceKind::UrlSnapshot | SourceKind::GitRepo => {
            if entry.snapshot_ref.is_none() {
                bail!(
                    "{} entry `{}` requires snapshotRef",
                    source.kind.label(),
                    source.id
                );
            }
        }
        SourceKind::ProjectFile
        | SourceKind::LocalRepo
        | SourceKind::PolicyDoc
        | SourceKind::ApiSpec
        | SourceKind::DesignReference => {
            if entry.path.is_none() {
                bail!(
                    "{} entry `{}` requires path",
                    source.kind.label(),
                    source.id
                );
            }
        }
    }
    Ok(())
}

fn validate_context_pack_id(id: &str) -> Result<()> {
    let Some(rest) = id.strip_prefix("CTX-") else {
        bail!("ContextPack id `{id}` must start with `CTX-`");
    };
    if rest.is_empty() || rest.contains('/') || rest.contains('\\') {
        bail!("ContextPack id `{id}` must be non-empty and path-safe");
    }
    Ok(())
}

fn validate_work_order_id(id: &str) -> Result<()> {
    let Some(number) = id.strip_prefix("WO-") else {
        bail!("workOrder `{id}` must start with `WO-`");
    };
    if number.is_empty() || !number.chars().all(|ch| ch.is_ascii_digit()) {
        bail!("workOrder `{id}` must use `WO-<digits>`");
    }
    Ok(())
}

fn ensure_source_id(field: &str, id: &str) -> Result<()> {
    ensure_nonempty(field, id)?;
    if id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        Ok(())
    } else {
        bail!("{field} `{id}` may only use ASCII letters, digits, '-', '_', or '.'")
    }
}

fn validate_roles(field: &str, roles: &[String]) -> Result<()> {
    if roles.is_empty() {
        bail!("{field} cannot be empty");
    }
    for role in roles {
        ensure_nonempty(field, role)?;
        if role.contains('/') || role.contains('\\') {
            bail!("{field} entry `{role}` must not contain path separators");
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
    use crate::source_registry::{RefreshPolicy, SourceTrustLevel};

    fn registry() -> SourceRegistry {
        SourceRegistry {
            schema_version: crate::source_registry::SOURCE_REGISTRY_SCHEMA_VERSION,
            sources: vec![
                ProjectSource {
                    id: "core-api".to_owned(),
                    kind: SourceKind::LocalRepo,
                    path: Some("../core-api".to_owned()),
                    url: None,
                    pin: "commit:abc123".to_owned(),
                    trust_level: SourceTrustLevel::Internal,
                    owner: "platform-team".to_owned(),
                    visible_roles: vec!["host".to_owned(), "engineer".to_owned()],
                    purpose: "Integration behavior.".to_owned(),
                    refresh_policy: RefreshPolicy::OnConfirmation,
                },
                ProjectSource {
                    id: "security-policy".to_owned(),
                    kind: SourceKind::PolicyDoc,
                    path: Some("docs/security.md".to_owned()),
                    url: None,
                    pin: "sha256:def456".to_owned(),
                    trust_level: SourceTrustLevel::Policy,
                    owner: "security".to_owned(),
                    visible_roles: vec!["host".to_owned(), "security".to_owned()],
                    purpose: "Security policy.".to_owned(),
                    refresh_policy: RefreshPolicy::Manual,
                },
            ],
        }
    }

    fn pack() -> ContextPack {
        ContextPack {
            schema_version: CONTEXT_PACK_SCHEMA_VERSION,
            id: "CTX-WO-0209".to_owned(),
            work_order: "WO-0209".to_owned(),
            entries: vec![
                ContextPackEntry {
                    source_id: "core-api".to_owned(),
                    path: Some("src/contracts.rs".to_owned()),
                    range: Some(ContextRange {
                        start_line: 10,
                        end_line: 40,
                    }),
                    snapshot_ref: None,
                    reason: "Engineer needs API contract definitions.".to_owned(),
                    target_roles: vec!["engineer".to_owned()],
                    source_pin: "commit:abc123".to_owned(),
                    trust_level: SourceTrustLevel::Internal,
                },
                ContextPackEntry {
                    source_id: "security-policy".to_owned(),
                    path: Some("docs/security.md".to_owned()),
                    range: None,
                    snapshot_ref: None,
                    reason: "Security needs policy constraints.".to_owned(),
                    target_roles: vec!["security".to_owned()],
                    source_pin: "sha256:def456".to_owned(),
                    trust_level: SourceTrustLevel::Policy,
                },
            ],
        }
    }

    #[test]
    fn context_pack_validates_against_registry_without_warnings() {
        let validation = pack()
            .validate_against_registry(&registry())
            .expect("valid pack");
        assert!(validation.warnings.is_empty());
    }

    #[test]
    fn stale_pin_is_warning_not_hidden() {
        let mut pack = pack();
        pack.entries[0].source_pin = "commit:old".to_owned();

        let validation = pack
            .validate_against_registry(&registry())
            .expect("stale pack still auditable");

        assert_eq!(validation.warnings.len(), 1);
        assert!(validation.warnings[0].contains("pin is stale"));
    }

    #[test]
    fn target_role_must_be_visible_for_source() {
        let mut pack = pack();
        pack.entries[0].target_roles = vec!["security".to_owned()];

        let err = pack
            .validate_against_registry(&registry())
            .expect_err("role not visible");

        assert!(err.to_string().contains("not visible"));
    }
}
