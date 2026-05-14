//! `.coderoom/priors.lock` — supply-chain guard for role priors (amendment A-008).
//!
//! Records, for every declared role, a component-level fingerprint of
//! the inputs that produce the composed system prompt: the built-in
//! kernel version, the shared `shared.md` body (if present), and each
//! role's `roles/<role>.md` priors file.
//!
//! Locking is component-level, not composition-level: per-day journal
//! entries and session-time patches are deliberately excluded so the
//! lockfile stays stable across day boundaries.
//!
//! Format: TOML, git-tracked. Schema kept simple so a human can diff
//! it in code review.
//!
//! Hash format: reuses [`crate::adapter::cc::fingerprint`]
//! (`DefaultHasher` with `dh1:` prefix). Stability across Rust
//! releases is not guaranteed; if a future release rotates the
//! algorithm, `cr doctor --fix` regenerates a fresh lock.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::{Config, CODEROOM_DIR, ROLES_DIR};
use crate::priors::{KERNEL_PROTOCOL, SHARED_FILE};

/// Filename of the lockfile under `.coderoom/`.
pub const LOCK_FILE: &str = "priors.lock";

/// Current lockfile schema version.
pub const LOCK_VERSION: u32 = 1;

/// On-disk representation of `.coderoom/priors.lock`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockFile {
    /// Schema version.
    pub version: u32,
    /// Fingerprint of the built-in kernel priors string at lock time.
    pub kernel_hash: String,
    /// Fingerprint of `.coderoom/shared.md` if present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_hash: Option<String>,
    /// One entry per declared role, sorted by name for stable diffs.
    pub roles: BTreeMap<String, RoleEntry>,
}

/// Lockfile entry for one role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleEntry {
    /// Fingerprint of `.coderoom/roles/<role>.md`.
    pub priors_hash: String,
}

/// Result of comparing the current on-disk priors against a stored lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriftReport {
    /// Drift entries, ordered: kernel, shared, then per-role.
    pub entries: Vec<DriftEntry>,
}

impl DriftReport {
    /// Whether the report has any drift entries.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.entries.is_empty()
    }
}

/// One drifted component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriftEntry {
    /// Component that drifted.
    pub component: Component,
    /// Kind of drift.
    pub kind: DriftKind,
}

/// Identifies the priors component a drift entry refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Component {
    /// Built-in CodeRoom kernel string.
    Kernel,
    /// `.coderoom/shared.md` project-wide priors.
    Shared,
    /// `.coderoom/roles/<role>.md` for the named role.
    Role(String),
}

/// Kinds of drift surfaced by [`diff`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriftKind {
    /// Component content hash differs between lock and current state.
    HashMismatch {
        /// Hash recorded in `priors.lock`.
        locked: String,
        /// Hash computed from the current on-disk content.
        current: String,
    },
    /// Lock recorded a value for this component but the current state
    /// has none.
    Removed {
        /// Hash recorded in `priors.lock`.
        locked: String,
    },
    /// Current state has a component the lock does not.
    Added {
        /// Hash computed from the current on-disk content.
        current: String,
    },
}

/// Compute the [`LockFile`] for the current on-disk priors.
pub fn compute_current(project_root: &Path) -> Result<LockFile> {
    let coderoom_dir = project_root.join(CODEROOM_DIR);
    let cfg = Config::load(project_root)
        .with_context(|| format!("loading config in {}", project_root.display()))?;

    let kernel_hash = crate::adapter::cc::fingerprint(KERNEL_PROTOCOL);

    let shared_path = coderoom_dir.join(SHARED_FILE);
    let shared_hash = if shared_path.is_file() {
        let body = std::fs::read_to_string(&shared_path)
            .with_context(|| format!("reading {}", shared_path.display()))?;
        Some(crate::adapter::cc::fingerprint(&body))
    } else {
        None
    };

    let mut roles = BTreeMap::new();
    for name in cfg.role_names() {
        let role_path = coderoom_dir.join(ROLES_DIR).join(format!("{name}.md"));
        let priors_hash = if role_path.is_file() {
            let body = std::fs::read_to_string(&role_path)
                .with_context(|| format!("reading {}", role_path.display()))?;
            crate::adapter::cc::fingerprint(&body)
        } else {
            String::new()
        };
        roles.insert(name.to_owned(), RoleEntry { priors_hash });
    }

    Ok(LockFile {
        version: LOCK_VERSION,
        kernel_hash,
        shared_hash,
        roles,
    })
}

/// Read `.coderoom/priors.lock` if it exists.
pub fn read(project_root: &Path) -> Result<Option<LockFile>> {
    let path = project_root.join(CODEROOM_DIR).join(LOCK_FILE);
    match std::fs::read_to_string(&path) {
        Ok(body) => {
            let lock: LockFile =
                toml::from_str(&body).with_context(|| format!("parsing {}", path.display()))?;
            Ok(Some(lock))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

/// Write `lock` to `.coderoom/priors.lock`.
pub fn write(project_root: &Path, lock: &LockFile) -> Result<()> {
    let coderoom_dir = project_root.join(CODEROOM_DIR);
    std::fs::create_dir_all(&coderoom_dir)
        .with_context(|| format!("creating {}", coderoom_dir.display()))?;
    let path = coderoom_dir.join(LOCK_FILE);
    let body =
        toml::to_string_pretty(lock).with_context(|| format!("serializing {}", path.display()))?;
    std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Compare `locked` against `current` and return a drift report.
#[must_use]
pub fn diff(locked: &LockFile, current: &LockFile) -> DriftReport {
    let mut entries = Vec::new();
    if locked.kernel_hash != current.kernel_hash {
        entries.push(DriftEntry {
            component: Component::Kernel,
            kind: DriftKind::HashMismatch {
                locked: locked.kernel_hash.clone(),
                current: current.kernel_hash.clone(),
            },
        });
    }
    match (&locked.shared_hash, &current.shared_hash) {
        (Some(l), Some(c)) if l != c => {
            entries.push(DriftEntry {
                component: Component::Shared,
                kind: DriftKind::HashMismatch {
                    locked: l.clone(),
                    current: c.clone(),
                },
            });
        }
        (Some(l), None) => {
            entries.push(DriftEntry {
                component: Component::Shared,
                kind: DriftKind::Removed { locked: l.clone() },
            });
        }
        (None, Some(c)) => {
            entries.push(DriftEntry {
                component: Component::Shared,
                kind: DriftKind::Added { current: c.clone() },
            });
        }
        _ => {}
    }

    let mut role_names: std::collections::BTreeSet<&String> = std::collections::BTreeSet::new();
    role_names.extend(locked.roles.keys());
    role_names.extend(current.roles.keys());
    for name in role_names {
        match (locked.roles.get(name), current.roles.get(name)) {
            (Some(l), Some(c)) if l.priors_hash != c.priors_hash => {
                entries.push(DriftEntry {
                    component: Component::Role(name.clone()),
                    kind: DriftKind::HashMismatch {
                        locked: l.priors_hash.clone(),
                        current: c.priors_hash.clone(),
                    },
                });
            }
            (Some(l), None) => {
                entries.push(DriftEntry {
                    component: Component::Role(name.clone()),
                    kind: DriftKind::Removed {
                        locked: l.priors_hash.clone(),
                    },
                });
            }
            (None, Some(c)) => {
                entries.push(DriftEntry {
                    component: Component::Role(name.clone()),
                    kind: DriftKind::Added {
                        current: c.priors_hash.clone(),
                    },
                });
            }
            _ => {}
        }
    }

    DriftReport { entries }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn tmp_project(roles: &[(&str, &str)], shared: Option<&str>) -> TempDir {
        let tmp = TempDir::new().unwrap();
        let coderoom = tmp.path().join(CODEROOM_DIR);
        fs::create_dir_all(coderoom.join(ROLES_DIR)).unwrap();
        let mut config_body = String::from("default_engine = \"cc\"\nhost_role = \"host\"\n");
        for (name, _) in roles {
            config_body.push_str(&format!("[roles.{name}]\nengine = \"cc\"\n"));
        }
        if !roles.iter().any(|(n, _)| *n == "host") {
            config_body.push_str("[roles.host]\nengine = \"cc\"\n");
            fs::write(coderoom.join(ROLES_DIR).join("host.md"), "# host\n").unwrap();
        }
        fs::write(coderoom.join("config.toml"), config_body).unwrap();
        for (name, body) in roles {
            fs::write(coderoom.join(ROLES_DIR).join(format!("{name}.md")), body).unwrap();
        }
        if let Some(body) = shared {
            fs::write(coderoom.join(SHARED_FILE), body).unwrap();
        }
        tmp
    }

    #[test]
    fn compute_current_hashes_kernel_shared_and_roles() {
        let tmp = tmp_project(&[("backend", "# backend body\n")], Some("# shared body\n"));
        let lock = compute_current(tmp.path()).unwrap();
        assert_eq!(lock.version, LOCK_VERSION);
        assert!(lock.kernel_hash.starts_with("dh1:"));
        assert!(lock.shared_hash.is_some());
        assert!(lock.roles.contains_key("backend"));
        assert!(lock.roles.contains_key("host"));
    }

    #[test]
    fn compute_current_no_shared_yields_none() {
        let tmp = tmp_project(&[], None);
        let lock = compute_current(tmp.path()).unwrap();
        assert!(lock.shared_hash.is_none());
    }

    #[test]
    fn write_and_read_round_trip() {
        let tmp = tmp_project(&[("backend", "# backend\n")], None);
        let lock = compute_current(tmp.path()).unwrap();
        write(tmp.path(), &lock).unwrap();
        let read_back = read(tmp.path()).unwrap().expect("lockfile present");
        assert_eq!(lock, read_back);
    }

    #[test]
    fn read_missing_returns_none() {
        let tmp = tmp_project(&[], None);
        assert!(read(tmp.path()).unwrap().is_none());
    }

    #[test]
    fn diff_detects_role_hash_change() {
        let mut locked = LockFile {
            version: LOCK_VERSION,
            kernel_hash: "dh1:k".into(),
            shared_hash: None,
            roles: BTreeMap::new(),
        };
        locked.roles.insert(
            "backend".into(),
            RoleEntry {
                priors_hash: "dh1:old".into(),
            },
        );
        let mut current = locked.clone();
        current.roles.insert(
            "backend".into(),
            RoleEntry {
                priors_hash: "dh1:new".into(),
            },
        );
        let report = diff(&locked, &current);
        assert_eq!(report.entries.len(), 1);
        assert_eq!(
            report.entries[0].component,
            Component::Role("backend".into())
        );
    }

    #[test]
    fn diff_detects_role_added_and_removed() {
        let mut locked = LockFile {
            version: LOCK_VERSION,
            kernel_hash: "dh1:k".into(),
            shared_hash: None,
            roles: BTreeMap::new(),
        };
        locked.roles.insert(
            "backend".into(),
            RoleEntry {
                priors_hash: "dh1:b".into(),
            },
        );
        let mut current = LockFile {
            version: LOCK_VERSION,
            kernel_hash: "dh1:k".into(),
            shared_hash: None,
            roles: BTreeMap::new(),
        };
        current.roles.insert(
            "security".into(),
            RoleEntry {
                priors_hash: "dh1:s".into(),
            },
        );
        let report = diff(&locked, &current);
        let components: Vec<_> = report.entries.iter().map(|e| e.component.clone()).collect();
        assert!(components.contains(&Component::Role("backend".into())));
        assert!(components.contains(&Component::Role("security".into())));
    }

    #[test]
    fn diff_returns_clean_for_identical_inputs() {
        let lock = LockFile {
            version: LOCK_VERSION,
            kernel_hash: "dh1:k".into(),
            shared_hash: Some("dh1:s".into()),
            roles: BTreeMap::new(),
        };
        assert!(diff(&lock, &lock).is_clean());
    }
}
