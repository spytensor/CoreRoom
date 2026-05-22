//! Role knowledge manifest helpers.
//!
//! The preferred role layout is:
//!
//! ```text
//! .coderoom/roles/<role>/
//! ├── priors.md
//! ├── knowledge/
//! └── .knowledge-manifest.toml
//! ```
//!
//! Legacy `.coderoom/roles/<role>.md` files remain readable. Mutating
//! knowledge commands migrate the legacy file into the directory layout.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{bail, Context, Result};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::ROLES_DIR;

/// File name for a role's base priors in the directory layout.
pub const ROLE_PRIORS_FILE: &str = "priors.md";

/// Directory under each role that stores mounted domain documents.
pub const KNOWLEDGE_DIR: &str = "knowledge";

/// TOML manifest file name under each role directory.
pub const KNOWLEDGE_MANIFEST_FILE: &str = ".knowledge-manifest.toml";

/// Supported mounted knowledge file extensions.
pub const KNOWLEDGE_EXTENSIONS: [&str; 2] = ["md", "txt"];

/// Maximum composed prompt size that only emits a warning.
pub const WARN_COMPOSED_PRIORS_BYTES: usize = 100 * 1024;

/// Maximum composed prompt size without an explicit override.
pub const MAX_COMPOSED_PRIORS_BYTES: usize = 500 * 1024;

/// Manifest schema stored in `.knowledge-manifest.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnowledgeManifest {
    /// Mounted files in deterministic composition order.
    #[serde(default)]
    pub files: Vec<KnowledgeEntry>,
}

/// One mounted knowledge document.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnowledgeEntry {
    /// File name under `knowledge/`.
    pub name: String,
    /// SHA-256 of the copied file bytes.
    pub sha256: String,
    /// UTC RFC3339 timestamp when the document was attached.
    pub attached_at: String,
}

/// Result of attaching a knowledge file to a role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachOutcome {
    /// Manifest entry that was written.
    pub entry: KnowledgeEntry,
    /// Destination path inside the role's `knowledge/` directory.
    pub path: PathBuf,
    /// Legacy priors path that was migrated, when applicable.
    pub migrated_legacy: Option<PathBuf>,
}

/// Result of detaching a knowledge file from a role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetachOutcome {
    /// Removed manifest entry.
    pub entry: KnowledgeEntry,
    /// Removed file path inside `knowledge/`.
    pub path: PathBuf,
    /// Whether the file existed and was removed.
    pub removed_file: bool,
}

/// Knowledge entry enriched with filesystem metadata for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeInventoryEntry {
    /// Manifest or scanned entry.
    pub entry: KnowledgeEntry,
    /// Path inside `knowledge/`.
    pub path: PathBuf,
    /// Last filesystem modification time, if the file exists.
    pub modified_at: Option<String>,
}

/// Return `.coderoom/roles/<role>`.
#[must_use]
pub fn role_dir(coderoom_dir: &Path, role: &str) -> PathBuf {
    coderoom_dir.join(ROLES_DIR).join(role)
}

/// Return `.coderoom/roles/<role>/priors.md`.
#[must_use]
pub fn preferred_role_priors_path(coderoom_dir: &Path, role: &str) -> PathBuf {
    role_dir(coderoom_dir, role).join(ROLE_PRIORS_FILE)
}

/// Return legacy `.coderoom/roles/<role>.md`.
#[must_use]
pub fn legacy_role_priors_path(coderoom_dir: &Path, role: &str) -> PathBuf {
    coderoom_dir.join(ROLES_DIR).join(format!("{role}.md"))
}

/// Return `.coderoom/roles/<role>/knowledge`.
#[must_use]
pub fn knowledge_dir(coderoom_dir: &Path, role: &str) -> PathBuf {
    role_dir(coderoom_dir, role).join(KNOWLEDGE_DIR)
}

/// Return the manifest path for a role.
#[must_use]
pub fn manifest_path_for_role_dir(role_dir: &Path) -> PathBuf {
    role_dir.join(KNOWLEDGE_MANIFEST_FILE)
}

/// Return the existing role priors path, preferring the new directory
/// layout and falling back to the legacy flat `.md` file.
#[must_use]
pub fn role_priors_path_existing(coderoom_dir: &Path, role: &str) -> Option<PathBuf> {
    let preferred = preferred_role_priors_path(coderoom_dir, role);
    if preferred.is_file() {
        return Some(preferred);
    }
    let legacy = legacy_role_priors_path(coderoom_dir, role);
    if legacy.is_file() {
        eprintln!(
            "warning: .coderoom/roles/{role}.md is deprecated; use .coderoom/roles/{role}/priors.md"
        );
        return Some(legacy);
    }
    None
}

/// Return the effective priors path for diagnostics. If no priors file
/// exists, returns the preferred directory-layout path.
#[must_use]
pub fn role_priors_path_for_config(coderoom_dir: &Path, role: &str) -> PathBuf {
    role_priors_path_existing(coderoom_dir, role)
        .unwrap_or_else(|| preferred_role_priors_path(coderoom_dir, role))
}

/// Create an empty role directory layout.
pub fn create_role_layout(coderoom_dir: &Path, role: &str, priors_body: &str) -> Result<PathBuf> {
    let dir = role_dir(coderoom_dir, role);
    let priors_path = dir.join(ROLE_PRIORS_FILE);
    std::fs::create_dir_all(dir.join(KNOWLEDGE_DIR))
        .with_context(|| format!("creating role layout {}", dir.display()))?;
    if !priors_path.exists() {
        std::fs::write(&priors_path, priors_body)
            .with_context(|| format!("writing {}", priors_path.display()))?;
    }
    Ok(priors_path)
}

/// Ensure a role is in the directory layout. Legacy priors are moved
/// into `priors.md` and the old flat file is removed.
pub fn ensure_role_dir_layout(coderoom_dir: &Path, role: &str) -> Result<AttachLayout> {
    let dir = role_dir(coderoom_dir, role);
    let priors_path = dir.join(ROLE_PRIORS_FILE);
    let legacy_path = legacy_role_priors_path(coderoom_dir, role);
    let mut migrated_legacy = None;

    if priors_path.is_file() {
        std::fs::create_dir_all(dir.join(KNOWLEDGE_DIR))
            .with_context(|| format!("creating {}", dir.join(KNOWLEDGE_DIR).display()))?;
        return Ok(AttachLayout {
            role_dir: dir,
            priors_path,
            migrated_legacy,
        });
    }

    if legacy_path.is_file() {
        std::fs::create_dir_all(dir.join(KNOWLEDGE_DIR))
            .with_context(|| format!("creating {}", dir.join(KNOWLEDGE_DIR).display()))?;
        std::fs::rename(&legacy_path, &priors_path).with_context(|| {
            format!(
                "migrating legacy priors {} to {}",
                legacy_path.display(),
                priors_path.display()
            )
        })?;
        migrated_legacy = Some(legacy_path);
        return Ok(AttachLayout {
            role_dir: dir,
            priors_path,
            migrated_legacy,
        });
    }

    bail!(
        "role `{role}` is missing priors at {} or {}",
        priors_path.display(),
        legacy_path.display()
    );
}

/// Directory-layout paths prepared for an attach operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachLayout {
    /// `.coderoom/roles/<role>`.
    pub role_dir: PathBuf,
    /// `.coderoom/roles/<role>/priors.md`.
    pub priors_path: PathBuf,
    /// Legacy priors path that was migrated, when applicable.
    pub migrated_legacy: Option<PathBuf>,
}

/// Read a role's knowledge manifest. Missing manifests are treated as empty.
pub fn read_manifest(role_dir: &Path) -> Result<KnowledgeManifest> {
    let path = manifest_path_for_role_dir(role_dir);
    if !path.is_file() {
        return Ok(KnowledgeManifest::default());
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("reading manifest {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("parsing manifest {}", path.display()))
}

/// Write a role's knowledge manifest.
pub fn write_manifest(role_dir: &Path, manifest: &KnowledgeManifest) -> Result<()> {
    let path = manifest_path_for_role_dir(role_dir);
    let body = toml::to_string_pretty(manifest)
        .with_context(|| format!("serializing manifest {}", path.display()))?;
    std::fs::write(&path, body).with_context(|| format!("writing manifest {}", path.display()))
}

/// Attach a local markdown or text file to a role's knowledge directory.
pub fn attach_knowledge(
    coderoom_dir: &Path,
    role: &str,
    source: &Path,
    alias: Option<&str>,
) -> Result<AttachOutcome> {
    if !source.is_file() {
        bail!("knowledge source file not found: {}", source.display());
    }
    validate_supported_file(source)?;
    let name = normalize_knowledge_name(source, alias)?;
    validate_knowledge_name(&name)?;

    let layout = ensure_role_dir_layout(coderoom_dir, role)?;
    let knowledge = layout.role_dir.join(KNOWLEDGE_DIR);
    std::fs::create_dir_all(&knowledge)
        .with_context(|| format!("creating knowledge dir {}", knowledge.display()))?;
    let dest = knowledge.join(&name);

    let source_canon = source.canonicalize().ok();
    let dest_canon = dest.canonicalize().ok();
    if source_canon.as_ref() != dest_canon.as_ref() {
        std::fs::copy(source, &dest)
            .with_context(|| format!("copying {} to {}", source.display(), dest.display()))?;
    }

    let entry = KnowledgeEntry {
        name,
        sha256: sha256_file(&dest)?,
        attached_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
    };
    let mut manifest = read_manifest(&layout.role_dir)?;
    manifest
        .files
        .retain(|existing| existing.name != entry.name);
    manifest.files.push(entry.clone());
    manifest.files.sort_by(|a, b| a.name.cmp(&b.name));
    write_manifest(&layout.role_dir, &manifest)?;

    Ok(AttachOutcome {
        entry,
        path: dest,
        migrated_legacy: layout.migrated_legacy,
    })
}

/// Detach a knowledge file by manifest name.
pub fn detach_knowledge(coderoom_dir: &Path, role: &str, name: &str) -> Result<DetachOutcome> {
    validate_knowledge_name(name)?;
    let dir = role_dir(coderoom_dir, role);
    if !dir.is_dir() {
        bail!("role `{role}` is not using the directory layout; attach knowledge first");
    }
    let mut manifest = read_manifest(&dir)?;
    let Some(index) = manifest.files.iter().position(|entry| entry.name == name) else {
        bail!("knowledge file `{name}` is not attached to @{role}");
    };
    let entry = manifest.files.remove(index);
    let path = dir.join(KNOWLEDGE_DIR).join(name);
    let removed_file = if path.exists() {
        std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        true
    } else {
        false
    };
    write_manifest(&dir, &manifest)?;
    Ok(DetachOutcome {
        entry,
        path,
        removed_file,
    })
}

/// List role knowledge entries, using manifest order when present and
/// scanning `knowledge/` alphabetically when no manifest exists.
pub fn knowledge_inventory(
    coderoom_dir: &Path,
    role: &str,
) -> Result<Vec<KnowledgeInventoryEntry>> {
    let dir = role_dir(coderoom_dir, role);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let knowledge = dir.join(KNOWLEDGE_DIR);
    if !knowledge.is_dir() {
        return Ok(Vec::new());
    }

    let entries = if manifest_path_for_role_dir(&dir).is_file() {
        read_manifest(&dir)?.files
    } else {
        scan_knowledge_dir(&knowledge)?
    };

    entries
        .into_iter()
        .map(|entry| {
            let path = knowledge.join(&entry.name);
            let modified_at = std::fs::metadata(&path)
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .map(format_system_time);
            Ok(KnowledgeInventoryEntry {
                entry,
                path,
                modified_at,
            })
        })
        .collect()
}

/// Scan `knowledge/` for supported files in alphabetical order and compute
/// synthetic manifest entries from current bytes.
pub fn scan_knowledge_dir(knowledge_dir: &Path) -> Result<Vec<KnowledgeEntry>> {
    if !knowledge_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths = std::fs::read_dir(knowledge_dir)
        .with_context(|| format!("reading {}", knowledge_dir.display()))?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_supported_knowledge_file(path))
        .collect::<Vec<_>>();
    paths.sort();

    paths
        .into_iter()
        .map(|path| {
            let name = path
                .file_name()
                .and_then(OsStr::to_str)
                .with_context(|| format!("reading knowledge filename {}", path.display()))?
                .to_owned();
            Ok(KnowledgeEntry {
                name,
                sha256: sha256_file(&path)?,
                attached_at: String::new(),
            })
        })
        .collect()
}

/// Compute the SHA-256 hex digest for a file.
pub fn sha256_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(hex_digest(&hasher.finalize()))
}

/// Format a [`SystemTime`] as UTC RFC3339 seconds.
#[must_use]
pub fn format_system_time(time: SystemTime) -> String {
    chrono::DateTime::<Utc>::from(time).to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Whether a path has a supported mounted-knowledge extension.
#[must_use]
pub fn is_supported_knowledge_file(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .map(str::to_ascii_lowercase)
        .is_some_and(|ext| KNOWLEDGE_EXTENSIONS.contains(&ext.as_str()))
}

fn validate_supported_file(path: &Path) -> Result<()> {
    if is_supported_knowledge_file(path) {
        Ok(())
    } else {
        bail!(
            "knowledge files must be markdown or text (.md or .txt): {}",
            path.display()
        )
    }
}

fn normalize_knowledge_name(source: &Path, alias: Option<&str>) -> Result<String> {
    let source_name = source
        .file_name()
        .and_then(OsStr::to_str)
        .with_context(|| format!("source path has no filename: {}", source.display()))?;
    let source_ext = source
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or_default();

    let raw = alias.unwrap_or(source_name).trim();
    if raw.is_empty() {
        bail!("knowledge alias must be non-empty");
    }
    let mut name = raw.to_owned();
    if Path::new(raw).extension().is_none() && !source_ext.is_empty() {
        name.push('.');
        name.push_str(source_ext);
    }
    Ok(name)
}

fn validate_knowledge_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("knowledge name must be non-empty");
    }
    let path = Path::new(name);
    if path.file_name().and_then(OsStr::to_str) != Some(name) {
        bail!("knowledge name `{name}` must be a plain filename");
    }
    if name.starts_with('.') {
        bail!("knowledge name `{name}` must not start with `.`");
    }
    if !is_supported_knowledge_file(path) {
        bail!("knowledge name `{name}` must end in .md or .txt");
    }
    Ok(())
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}
