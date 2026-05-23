//! `priors.lock` generation and verification.
//!
//! The lock file is intentionally project-local and git-tracked. It records
//! stable SHA-256 digests for the files that define each role's prompt identity
//! plus a composite SHA for the fully composed prompt used at spawn time.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::{Config, CODEROOM_DIR};
use crate::priors::{self, ComposeOptions};

/// File name of the priors lock inside `.coderoom/`.
pub const LOCK_FILE: &str = "priors.lock";

const LOCK_VERSION: u32 = 1;

/// On-disk lock-file schema.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PriorsLock {
    /// Lock schema version.
    pub version: u32,
    /// Per-role lock entries, sorted by role name on write.
    pub roles: BTreeMap<String, LockedRole>,
}

/// Per-role lock data.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LockedRole {
    /// Composite SHA of the fully composed prompt for this role.
    pub composite: String,
    /// Ordered layer digests that define this role's prompt identity.
    pub layers: Vec<LockedLayer>,
}

/// One locked composition layer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LockedLayer {
    /// Layer class, for example `kernel`, `shared`, `role`, or `knowledge`.
    pub kind: String,
    /// Stable source label or project-relative source path.
    pub path: String,
    /// Raw SHA-256 hex digest of the layer content.
    pub sha256: String,
}

/// Verification result for `.coderoom/priors.lock`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyReport {
    /// Lock path that was checked.
    pub lock_path: PathBuf,
    /// Human-readable drift messages. Empty means the lock is current.
    pub drifts: Vec<String>,
}

impl VerifyReport {
    /// Whether verification found no drift.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.drifts.is_empty()
    }
}

/// Return the lock-file path for a `.coderoom/` directory.
#[must_use]
pub fn lock_path(coderoom_dir: &Path) -> PathBuf {
    coderoom_dir.join(LOCK_FILE)
}

/// Generate the current lock data for every configured role.
pub fn generate(coderoom_dir: &Path, options: ComposeOptions) -> Result<PriorsLock> {
    let project_root = coderoom_dir
        .parent()
        .map_or_else(|| Path::new(".").to_path_buf(), Path::to_path_buf);
    let cfg = Config::load(&project_root)
        .with_context(|| format!("loading config in {}", project_root.display()))?;
    generate_for_config(coderoom_dir, &cfg, options)
}

/// Generate lock data for an already-loaded config.
pub fn generate_for_config(
    coderoom_dir: &Path,
    cfg: &Config,
    options: ComposeOptions,
) -> Result<PriorsLock> {
    let mut role_names: Vec<&str> = cfg.role_names().collect();
    role_names.sort_unstable();

    let mut roles = BTreeMap::new();
    for role in role_names {
        let composed = priors::compose_for_with_options(coderoom_dir, role, options)
            .with_context(|| format!("composing priors for @{role}"))?;
        let layers = priors::lock_layers_for_role(coderoom_dir, role)
            .with_context(|| format!("collecting priors lock layers for @{role}"))?
            .into_iter()
            .map(|layer| LockedLayer {
                kind: layer.kind,
                path: layer.path,
                sha256: layer.sha256,
            })
            .collect();
        roles.insert(
            role.to_owned(),
            LockedRole {
                composite: priors::composite_hash(&composed),
                layers,
            },
        );
    }

    Ok(PriorsLock {
        version: LOCK_VERSION,
        roles,
    })
}

/// Write a freshly generated lock file.
pub fn write(coderoom_dir: &Path, options: ComposeOptions) -> Result<PriorsLock> {
    let lock = generate(coderoom_dir, options)?;
    write_lock(coderoom_dir, &lock)?;
    Ok(lock)
}

/// Write `lock` to `.coderoom/priors.lock`.
pub fn write_lock(coderoom_dir: &Path, lock: &PriorsLock) -> Result<()> {
    let path = lock_path(coderoom_dir);
    let rendered = toml::to_string_pretty(lock).context("serializing priors lock")?;
    std::fs::write(&path, rendered).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Read `.coderoom/priors.lock`.
pub fn read(coderoom_dir: &Path) -> Result<PriorsLock> {
    let path = lock_path(coderoom_dir);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

/// Verify lock entries against current on-disk priors content.
pub fn verify(coderoom_dir: &Path, options: ComposeOptions) -> Result<VerifyReport> {
    let path = lock_path(coderoom_dir);
    if !path.is_file() {
        return Ok(VerifyReport {
            lock_path: path,
            drifts: vec![format!(
                "{} is missing; run `cr lock` to create it",
                display_from_coderoom(coderoom_dir, LOCK_FILE)
            )],
        });
    }

    let expected = read(coderoom_dir)?;
    let current = generate(coderoom_dir, options)?;
    Ok(compare_locks(coderoom_dir, &expected, &current))
}

fn compare_locks(coderoom_dir: &Path, expected: &PriorsLock, current: &PriorsLock) -> VerifyReport {
    let mut drifts = Vec::new();
    if expected.version != current.version {
        drifts.push(format!(
            "lock version drift: expected {}, found {}",
            expected.version, current.version
        ));
    }

    let expected_roles = expected.roles.keys().collect::<BTreeSet<_>>();
    let current_roles = current.roles.keys().collect::<BTreeSet<_>>();
    for role in expected_roles.difference(&current_roles) {
        drifts.push(format!(
            "role @{role} is in priors.lock but not current config"
        ));
    }
    for role in current_roles.difference(&expected_roles) {
        drifts.push(format!("role @{role} is missing from priors.lock"));
    }
    for role in expected_roles.intersection(&current_roles) {
        let expected_role = &expected.roles[*role];
        let current_role = &current.roles[*role];
        if expected_role.composite != current_role.composite {
            drifts.push(format!(
                "role @{role} composite drift: expected {}, found {}",
                expected_role.composite, current_role.composite
            ));
        }
        compare_layers(
            &mut drifts,
            role,
            &expected_role.layers,
            &current_role.layers,
        );
    }

    VerifyReport {
        lock_path: lock_path(coderoom_dir),
        drifts,
    }
}

fn compare_layers(
    drifts: &mut Vec<String>,
    role: &str,
    expected: &[LockedLayer],
    current: &[LockedLayer],
) {
    let expected_layers = layer_map(expected);
    let current_layers = layer_map(current);
    let expected_keys = expected_layers.keys().collect::<BTreeSet<_>>();
    let current_keys = current_layers.keys().collect::<BTreeSet<_>>();

    for key in expected_keys.difference(&current_keys) {
        drifts.push(format!(
            "role @{role} layer {key} missing from current priors"
        ));
    }
    for key in current_keys.difference(&expected_keys) {
        drifts.push(format!("role @{role} layer {key} missing from priors.lock"));
    }
    for key in expected_keys.intersection(&current_keys) {
        let expected_layer = expected_layers[*key];
        let current_layer = current_layers[*key];
        if expected_layer.sha256 != current_layer.sha256 {
            drifts.push(format!(
                "role @{role} layer {} drift: expected {}, found {}",
                key, expected_layer.sha256, current_layer.sha256
            ));
        }
    }
}

fn layer_map(layers: &[LockedLayer]) -> BTreeMap<String, &LockedLayer> {
    layers
        .iter()
        .map(|layer| (format!("{} {}", layer.kind, layer.path), layer))
        .collect()
}

fn display_from_coderoom(coderoom_dir: &Path, name: &str) -> String {
    let file = coderoom_dir.join(name);
    if coderoom_dir.file_name().and_then(|s| s.to_str()) == Some(CODEROOM_DIR) {
        format!(".coderoom/{name}")
    } else {
        file.display().to_string()
    }
}
