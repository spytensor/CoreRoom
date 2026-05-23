//! Local priors-liveness telemetry.
//!
//! Liveness is intentionally per-user state: it records which prompt
//! segments were loaded into a role turn, without changing project priors or
//! shared lock files.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use crate::manifest;
use crate::priors::PriorsLayer;

/// Directory under `.coreroom/` containing per-role liveness sidecars.
pub const LIVENESS_DIR: &str = "liveness";

/// Default stale threshold for `cr doctor`.
pub const DEFAULT_STALE_DAYS: i64 = 30;

const LIVENESS_VERSION: u32 = 1;

/// Per-role liveness document stored as JSON.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoleLiveness {
    /// Schema version.
    pub version: u32,
    /// Role this sidecar belongs to.
    pub role: String,
    /// Segment liveness keyed by stable source path.
    #[serde(default)]
    pub segments: BTreeMap<String, SegmentLiveness>,
}

/// Liveness data for one composed priors segment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SegmentLiveness {
    /// Layer class, for example `shared`, `role`, or `knowledge`.
    pub kind: String,
    /// Stable project-relative source path.
    pub path: String,
    /// Current SHA-256 hex digest of the segment content.
    pub sha256: String,
    /// When this segment first entered local liveness tracking.
    pub attached_at: String,
    /// Last deterministic citation timestamp, if citation tracking exists.
    pub last_cited_at: Option<String>,
    /// Last deterministic match timestamp. Today this falls back to load time.
    pub last_matched_at: Option<String>,
    /// Count of deterministic hits. Today each loaded segment counts as a hit.
    pub hit_count: u64,
}

/// Stale liveness candidate reported by `cr doctor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleSegment {
    /// Role owning the segment.
    pub role: String,
    /// Segment kind.
    pub kind: String,
    /// Segment source path.
    pub path: String,
    /// Total observed hit count.
    pub hit_count: u64,
    /// Segment attachment timestamp.
    pub attached_at: String,
    /// Last match/load timestamp.
    pub last_matched_at: Option<String>,
    /// Last citation timestamp.
    pub last_cited_at: Option<String>,
    /// Suggested user command or edit action.
    pub recommendation: String,
}

impl RoleLiveness {
    fn empty(role: &str) -> Self {
        Self {
            version: LIVENESS_VERSION,
            role: role.to_owned(),
            segments: BTreeMap::new(),
        }
    }
}

/// Return `.coreroom/liveness/<role>.json`.
#[must_use]
pub fn path_for_role(coreroom_dir: &Path, role: &str) -> PathBuf {
    coreroom_dir.join(LIVENESS_DIR).join(format!("{role}.json"))
}

/// Read a role's liveness sidecar. Missing files return an empty document.
pub fn read(coreroom_dir: &Path, role: &str) -> Result<RoleLiveness> {
    let path = path_for_role(coreroom_dir, role);
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(RoleLiveness::empty(role)),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

/// Write a role's liveness sidecar.
pub fn write(coreroom_dir: &Path, doc: &RoleLiveness) -> Result<()> {
    let path = path_for_role(coreroom_dir, &doc.role);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let body = format!("{}\n", serde_json::to_string_pretty(doc)?);
    std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))
}

/// Record loaded priors layers for a role turn.
///
/// Fine-grained model citation tracking is not available yet, so this uses
/// the deterministic fallback from A-010: every segment loaded into the role
/// prompt counts as matched for this turn.
pub fn record_loaded(coreroom_dir: &Path, role: &str, layers: &[PriorsLayer]) -> Result<()> {
    let now = now_rfc3339();
    let mut doc = read(coreroom_dir, role)?;
    doc.version = LIVENESS_VERSION;
    role.clone_into(&mut doc.role);

    for layer in layers.iter().filter(|layer| layer.kind != "kernel") {
        let attached_at = doc
            .segments
            .get(&layer.path)
            .map(|segment| segment.attached_at.clone())
            .or_else(|| attached_at_from_manifest(coreroom_dir, role, layer))
            .unwrap_or_else(|| now.clone());
        let segment = doc
            .segments
            .entry(layer.path.clone())
            .or_insert_with(|| SegmentLiveness {
                kind: layer.kind.clone(),
                path: layer.path.clone(),
                sha256: layer.sha256.clone(),
                attached_at,
                last_cited_at: None,
                last_matched_at: None,
                hit_count: 0,
            });
        segment.kind.clone_from(&layer.kind);
        segment.path.clone_from(&layer.path);
        segment.sha256.clone_from(&layer.sha256);
        segment.last_matched_at = Some(now.clone());
        segment.hit_count = segment.hit_count.saturating_add(1);
    }

    write(coreroom_dir, &doc)
}

/// Return stale liveness segments across all local sidecars.
pub fn stale_segments(coreroom_dir: &Path, stale_days: i64) -> Result<Vec<StaleSegment>> {
    let dir = coreroom_dir.join(LIVENESS_DIR);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let cutoff = Utc::now() - Duration::days(stale_days.max(0));
    let mut stale = Vec::new();
    for entry in std::fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let doc: RoleLiveness =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        stale.extend(stale_segments_for_doc(&doc, cutoff));
    }
    stale.sort_by(|a, b| (&a.role, &a.path).cmp(&(&b.role, &b.path)));
    Ok(stale)
}

/// Return the liveness segment key for a mounted knowledge document.
#[must_use]
pub fn knowledge_segment_path(role: &str, name: &str) -> String {
    format!(".coreroom/roles/{role}/knowledge/{name}")
}

fn stale_segments_for_doc(doc: &RoleLiveness, cutoff: DateTime<Utc>) -> Vec<StaleSegment> {
    doc.segments
        .values()
        .filter(|segment| segment.kind != "kernel")
        .filter(|segment| segment_last_seen(segment).is_some_and(|last| last < cutoff))
        .map(|segment| StaleSegment {
            role: doc.role.clone(),
            kind: segment.kind.clone(),
            path: segment.path.clone(),
            hit_count: segment.hit_count,
            attached_at: segment.attached_at.clone(),
            last_matched_at: segment.last_matched_at.clone(),
            last_cited_at: segment.last_cited_at.clone(),
            recommendation: recommendation_for(&doc.role, segment),
        })
        .collect()
}

fn segment_last_seen(segment: &SegmentLiveness) -> Option<DateTime<Utc>> {
    let activity = [
        parse_rfc3339(segment.last_cited_at.as_deref()),
        parse_rfc3339(segment.last_matched_at.as_deref()),
    ]
    .into_iter()
    .flatten()
    .max();
    activity.or_else(|| parse_rfc3339(Some(&segment.attached_at)))
}

fn recommendation_for(role: &str, segment: &SegmentLiveness) -> String {
    if segment.kind == "knowledge" {
        let name = segment
            .path
            .rsplit('/')
            .next()
            .unwrap_or(segment.path.as_str());
        format!("cr role detach {role} {name}")
    } else {
        format!("review {}", segment.path)
    }
}

fn attached_at_from_manifest(
    coreroom_dir: &Path,
    role: &str,
    layer: &PriorsLayer,
) -> Option<String> {
    if layer.kind != "knowledge" {
        return None;
    }
    let name = layer.path.rsplit('/').next()?;
    manifest::knowledge_inventory(coreroom_dir, role)
        .ok()?
        .into_iter()
        .find(|item| item.entry.name == name)
        .map(|item| item.entry.attached_at)
}

fn parse_rfc3339(value: Option<&str>) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value?)
        .ok()
        .map(DateTime::from)
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}
