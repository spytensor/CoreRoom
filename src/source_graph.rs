//! Project source graph for multi-repo and external-context work.
//!
//! The Source Registry records individual sources. The Source Graph records how
//! those sources relate to each other for a WorkOrder, which exact pins were
//! used, which roles may see them, and whether context has drifted.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::CODEROOM_DIR;
use crate::context_pack::ContextPack;
use crate::source_registry::{ProjectSource, SourceKind, SourceTrustLevel};

/// File inside `.coderoom/` that stores the source graph.
pub const SOURCE_GRAPH_FILE: &str = "source-graph.toml";

/// Current persisted Source Graph schema version.
pub const SOURCE_GRAPH_SCHEMA_VERSION: u32 = 1;

/// Source graph node category.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum SourceGraphNodeKind {
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
    /// Operational runbook.
    Runbook,
    /// Release checklist or release readiness reference.
    ReleaseChecklist,
}

impl SourceGraphNodeKind {
    /// Stable persisted label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::ProjectFile => "project-file",
            Self::LocalRepo => "local-repo",
            Self::GitRepo => "git-repo",
            Self::UrlSnapshot => "url-snapshot",
            Self::PolicyDoc => "policy-doc",
            Self::ApiSpec => "api-spec",
            Self::DesignReference => "design-reference",
            Self::Runbook => "runbook",
            Self::ReleaseChecklist => "release-checklist",
        }
    }

    const fn pin_drift_kind(self, pin: &str) -> SourceGraphFindingKind {
        match self {
            Self::LocalRepo | Self::GitRepo if starts_with_const(pin, "commit:") => {
                SourceGraphFindingKind::CommitChanged
            }
            Self::UrlSnapshot => SourceGraphFindingKind::UrlSnapshotStale,
            _ if starts_with_const(pin, "sha256:") => SourceGraphFindingKind::FileHashChanged,
            _ => SourceGraphFindingKind::PinChanged,
        }
    }
}

/// Directed relationship between two graph nodes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum SourceGraphRelation {
    /// Source depends on another source.
    DependsOn,
    /// Source documents another source.
    Documents,
    /// Source constrains another source.
    Constrains,
    /// Source implements another source's contract.
    Implements,
    /// Source verifies another source.
    Verifies,
    /// Source is part of release readiness for another source.
    ReleasesWith,
}

impl SourceGraphRelation {
    /// Stable persisted label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::DependsOn => "depends-on",
            Self::Documents => "documents",
            Self::Constrains => "constrains",
            Self::Implements => "implements",
            Self::Verifies => "verifies",
            Self::ReleasesWith => "releases-with",
        }
    }
}

/// One source node in the graph.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceGraphNode {
    /// Source Registry id.
    pub source_id: String,
    /// Graph-specific source category.
    pub kind: SourceGraphNodeKind,
    /// Pinned identity, such as `sha256:<hex>`, `commit:<sha>`, or `snapshot:<sha>`.
    pub pin: String,
    /// Trust level.
    pub trust_level: SourceTrustLevel,
    /// Human/team owner for freshness and correctness questions.
    pub owner: String,
    /// Roles allowed to see this source by default.
    pub visible_roles: Vec<String>,
    /// Why this source appears in the graph.
    pub purpose: String,
    /// WorkOrders that used or should use this source.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub work_orders: Vec<String>,
    /// Local path, path inside a repository, or generated snapshot path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Remote URL, if applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Snapshot reference for URL snapshots and archived remote docs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_ref: Option<String>,
}

impl SourceGraphNode {
    /// Build a graph node from a registered project source.
    pub fn from_project_source(
        source: &ProjectSource,
        kind: SourceGraphNodeKind,
        work_orders: Vec<String>,
    ) -> Self {
        Self {
            source_id: source.id.clone(),
            kind,
            pin: source.pin.clone(),
            trust_level: source.trust_level,
            owner: source.owner.clone(),
            visible_roles: source.visible_roles.clone(),
            purpose: source.purpose.clone(),
            work_orders,
            path: source.path.clone(),
            url: source.url.clone(),
            snapshot_ref: None,
        }
    }

    /// Infer a graph node kind from a Source Registry kind.
    pub const fn kind_from_source_kind(kind: SourceKind) -> SourceGraphNodeKind {
        match kind {
            SourceKind::ProjectFile => SourceGraphNodeKind::ProjectFile,
            SourceKind::LocalRepo => SourceGraphNodeKind::LocalRepo,
            SourceKind::GitRepo => SourceGraphNodeKind::GitRepo,
            SourceKind::UrlSnapshot => SourceGraphNodeKind::UrlSnapshot,
            SourceKind::PolicyDoc => SourceGraphNodeKind::PolicyDoc,
            SourceKind::ApiSpec => SourceGraphNodeKind::ApiSpec,
            SourceKind::DesignReference => SourceGraphNodeKind::DesignReference,
        }
    }

    /// Validate graph node shape.
    pub fn validate(&self) -> Result<()> {
        ensure_source_id("sourceId", &self.source_id)?;
        ensure_nonempty("pin", &self.pin)?;
        ensure_nonempty("owner", &self.owner)?;
        ensure_nonempty("purpose", &self.purpose)?;
        validate_roles("visibleRoles", &self.visible_roles)?;
        for work_order in &self.work_orders {
            validate_work_order_ref(work_order)?;
        }
        if self.kind == SourceGraphNodeKind::UrlSnapshot {
            if self.url.as_deref().is_none_or(str::is_empty) {
                bail!("url-snapshot source `{}` requires url", self.source_id);
            }
            if self.snapshot_ref.as_deref().is_none_or(str::is_empty) {
                bail!(
                    "url-snapshot source `{}` requires snapshotRef",
                    self.source_id
                );
            }
        }
        Ok(())
    }
}

/// One edge between source graph nodes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceGraphEdge {
    /// Source id for the dependent or describing node.
    pub from: String,
    /// Source id for the dependency or described node.
    pub to: String,
    /// Relationship kind.
    pub relation: SourceGraphRelation,
    /// Why this relationship matters.
    pub reason: String,
    /// Relationship owner.
    pub owner: String,
    /// Trust level for this relationship.
    pub trust_level: SourceTrustLevel,
    /// WorkOrders that used or should use this edge.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub work_orders: Vec<String>,
}

impl SourceGraphEdge {
    /// Validate edge shape and endpoint ids.
    pub fn validate_shape(&self) -> Result<()> {
        ensure_source_id("from", &self.from)?;
        ensure_source_id("to", &self.to)?;
        if self.from == self.to {
            bail!("source graph edge `{}` cannot point to itself", self.from);
        }
        ensure_nonempty("reason", &self.reason)?;
        ensure_nonempty("owner", &self.owner)?;
        for work_order in &self.work_orders {
            validate_work_order_ref(work_order)?;
        }
        Ok(())
    }
}

/// Persisted source graph.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceGraph {
    /// Schema version.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Source nodes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<SourceGraphNode>,
    /// Source edges.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<SourceGraphEdge>,
}

impl SourceGraph {
    /// Create an empty source graph.
    pub const fn new() -> Self {
        Self {
            schema_version: SOURCE_GRAPH_SCHEMA_VERSION,
            nodes: Vec::new(),
            edges: Vec::new(),
        }
    }

    /// Validate graph shape.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SOURCE_GRAPH_SCHEMA_VERSION {
            bail!(
                "unsupported Source Graph schemaVersion {}; expected {}",
                self.schema_version,
                SOURCE_GRAPH_SCHEMA_VERSION
            );
        }
        let mut ids = BTreeSet::new();
        for node in &self.nodes {
            node.validate()?;
            if !ids.insert(node.source_id.as_str()) {
                bail!("duplicate source graph node `{}`", node.source_id);
            }
        }
        for edge in &self.edges {
            edge.validate_shape()?;
            if !ids.contains(edge.from.as_str()) {
                bail!("source graph edge from `{}` is not registered", edge.from);
            }
            if !ids.contains(edge.to.as_str()) {
                bail!("source graph edge to `{}` is not registered", edge.to);
            }
        }
        Ok(())
    }

    /// Return node by source id.
    pub fn node(&self, source_id: &str) -> Option<&SourceGraphNode> {
        self.nodes.iter().find(|node| node.source_id == source_id)
    }

    /// Detect drift by comparing pinned graph facts against current facts.
    pub fn detect_drift(
        &self,
        current_facts: &BTreeMap<String, SourceGraphNodeFacts>,
    ) -> Result<Vec<SourceGraphFinding>> {
        self.validate()?;
        let mut findings = Vec::new();
        for node in &self.nodes {
            let Some(facts) = current_facts.get(&node.source_id) else {
                findings.push(SourceGraphFinding::new(
                    node.source_id.clone(),
                    SourceGraphFindingKind::MissingSource,
                    format!("source `{}` has no current facts", node.source_id),
                ));
                continue;
            };
            if !facts.exists {
                findings.push(SourceGraphFinding::new(
                    node.source_id.clone(),
                    SourceGraphFindingKind::MissingSource,
                    format!("source `{}` is missing", node.source_id),
                ));
                continue;
            }
            if facts.current_pin != node.pin {
                findings.push(SourceGraphFinding::new(
                    node.source_id.clone(),
                    node.kind.pin_drift_kind(&node.pin),
                    format!(
                        "source `{}` pin drifted from `{}` to `{}`",
                        node.source_id, node.pin, facts.current_pin
                    ),
                ));
            }
            if facts.trust_level != node.trust_level {
                findings.push(SourceGraphFinding::new(
                    node.source_id.clone(),
                    SourceGraphFindingKind::TrustChanged,
                    format!(
                        "source `{}` trust changed from `{}` to `{}`",
                        node.source_id,
                        node.trust_level.label(),
                        facts.trust_level.label()
                    ),
                ));
            }
            if role_set(&facts.visible_roles) != role_set(&node.visible_roles) {
                findings.push(SourceGraphFinding::new(
                    node.source_id.clone(),
                    SourceGraphFindingKind::VisibilityChanged,
                    format!("source `{}` role visibility changed", node.source_id),
                ));
            }
        }
        Ok(findings)
    }

    /// Validate ContextPack source ids and role visibility against the graph.
    pub fn validate_context_pack_visibility(
        &self,
        pack: &ContextPack,
    ) -> Result<SourceGraphContextReport> {
        self.validate()?;
        let mut findings = Vec::new();
        for entry in &pack.entries {
            let Some(node) = self.node(&entry.source_id) else {
                findings.push(SourceGraphFinding::new(
                    entry.source_id.clone(),
                    SourceGraphFindingKind::MissingSource,
                    format!(
                        "ContextPack `{}` references missing graph node `{}`",
                        pack.id, entry.source_id
                    ),
                ));
                continue;
            };
            for role in &entry.target_roles {
                if !node.visible_roles.iter().any(|visible| visible == role) {
                    findings.push(SourceGraphFinding::new(
                        entry.source_id.clone(),
                        SourceGraphFindingKind::VisibilityDenied,
                        format!(
                            "role `{role}` is not visible for source `{}` in ContextPack `{}`",
                            entry.source_id, pack.id
                        ),
                    ));
                }
            }
        }
        Ok(SourceGraphContextReport { findings })
    }

    /// Render a host-facing explanation of source versions used for a WorkOrder.
    pub fn explain_work_order_sources(&self, work_order: &str) -> Result<String> {
        validate_work_order_ref(work_order)?;
        self.validate()?;

        let nodes = self
            .nodes
            .iter()
            .filter(|node| node.work_orders.iter().any(|item| item == work_order))
            .collect::<Vec<_>>();
        let edges = self
            .edges
            .iter()
            .filter(|edge| edge.work_orders.iter().any(|item| item == work_order))
            .collect::<Vec<_>>();

        let mut out = String::new();
        let _ = writeln!(out, "Source versions for {work_order}");
        if nodes.is_empty() {
            let _ = writeln!(out, "sources: none");
        } else {
            let _ = writeln!(out, "sources:");
            for node in nodes {
                let _ = writeln!(
                    out,
                    "- {} ({}, pin {}, trust {}, roles {})",
                    node.source_id,
                    node.kind.label(),
                    node.pin,
                    node.trust_level.label(),
                    node.visible_roles.join(", ")
                );
            }
        }
        if edges.is_empty() {
            let _ = writeln!(out, "graphPaths: none");
        } else {
            let _ = writeln!(out, "graphPaths:");
            for edge in edges {
                let _ = writeln!(
                    out,
                    "- {} -> {} ({}) - {}",
                    edge.from,
                    edge.to,
                    edge.relation.label(),
                    edge.reason
                );
            }
        }
        Ok(out)
    }

    /// Generate source citations suitable for Evidence Packet summaries.
    pub fn evidence_citations_for_work_order(
        &self,
        work_order: &str,
    ) -> Result<Vec<SourceGraphEvidenceCitation>> {
        validate_work_order_ref(work_order)?;
        self.validate()?;
        let mut citations = Vec::new();
        for node in self
            .nodes
            .iter()
            .filter(|node| node.work_orders.iter().any(|item| item == work_order))
        {
            let paths = self
                .edges
                .iter()
                .filter(|edge| {
                    edge.work_orders.iter().any(|item| item == work_order)
                        && (edge.from == node.source_id || edge.to == node.source_id)
                })
                .map(|edge| format!("{}->{}:{}", edge.from, edge.to, edge.relation.label()))
                .collect::<Vec<_>>();
            citations.push(SourceGraphEvidenceCitation {
                source_id: node.source_id.clone(),
                pin: node.pin.clone(),
                graph_paths: paths,
                reason: node.purpose.clone(),
            });
        }
        Ok(citations)
    }
}

impl Default for SourceGraph {
    fn default() -> Self {
        Self::new()
    }
}

/// Current facts for one source graph node.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceGraphNodeFacts {
    /// Whether the source exists and is readable/reachable in the current environment.
    pub exists: bool,
    /// Current pin observed from the source.
    pub current_pin: String,
    /// Current trust level.
    pub trust_level: SourceTrustLevel,
    /// Current visible roles.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub visible_roles: Vec<String>,
}

/// Finding category for source graph checks.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum SourceGraphFindingKind {
    /// Source is missing or has no current facts.
    MissingSource,
    /// Git commit pin changed.
    CommitChanged,
    /// File hash pin changed.
    FileHashChanged,
    /// URL snapshot pin changed.
    UrlSnapshotStale,
    /// Pin changed but the pin type is not classified.
    PinChanged,
    /// Trust level changed.
    TrustChanged,
    /// Role visibility changed.
    VisibilityChanged,
    /// ContextPack asks to show a source to a role that is not allowed.
    VisibilityDenied,
}

impl SourceGraphFindingKind {
    /// Stable persisted label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::MissingSource => "missing-source",
            Self::CommitChanged => "commit-changed",
            Self::FileHashChanged => "file-hash-changed",
            Self::UrlSnapshotStale => "url-snapshot-stale",
            Self::PinChanged => "pin-changed",
            Self::TrustChanged => "trust-changed",
            Self::VisibilityChanged => "visibility-changed",
            Self::VisibilityDenied => "visibility-denied",
        }
    }
}

/// Source graph finding.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceGraphFinding {
    /// Source id.
    pub source_id: String,
    /// Finding kind.
    pub kind: SourceGraphFindingKind,
    /// Human-facing message.
    pub message: String,
}

impl SourceGraphFinding {
    /// Create a finding.
    pub fn new(
        source_id: impl Into<String>,
        kind: SourceGraphFindingKind,
        message: impl Into<String>,
    ) -> Self {
        Self {
            source_id: source_id.into(),
            kind,
            message: message.into(),
        }
    }
}

/// ContextPack visibility report against a Source Graph.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SourceGraphContextReport {
    /// Findings for the ContextPack.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<SourceGraphFinding>,
}

/// Confirmation-required source refresh plan.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceRefreshPlan {
    /// Source id.
    pub source_id: String,
    /// Existing pin.
    pub current_pin: String,
    /// Proposed replacement pin.
    pub proposed_pin: String,
    /// Host must ask the user before applying this plan.
    pub requires_confirmation: bool,
    /// Silent refresh is never allowed.
    pub allows_silent_refresh: bool,
}

impl SourceRefreshPlan {
    /// Build a refresh plan for a source pin change.
    pub fn new(
        source_id: impl Into<String>,
        current_pin: impl Into<String>,
        proposed_pin: impl Into<String>,
    ) -> Result<Self> {
        let source_id = source_id.into();
        let current_pin = current_pin.into();
        let proposed_pin = proposed_pin.into();
        ensure_source_id("sourceId", &source_id)?;
        ensure_nonempty("currentPin", &current_pin)?;
        ensure_nonempty("proposedPin", &proposed_pin)?;
        if current_pin == proposed_pin {
            bail!("proposedPin must differ from currentPin for source `{source_id}`");
        }
        Ok(Self {
            source_id,
            current_pin,
            proposed_pin,
            requires_confirmation: true,
            allows_silent_refresh: false,
        })
    }

    /// Convert this plan into a confirmed refresh.
    pub fn confirm(self, confirmed_by: impl Into<String>) -> Result<ConfirmedSourceRefresh> {
        let confirmed_by = confirmed_by.into();
        ensure_nonempty("confirmedBy", &confirmed_by)?;
        Ok(ConfirmedSourceRefresh {
            plan: self,
            confirmed_by,
        })
    }
}

/// Confirmed source refresh.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmedSourceRefresh {
    /// Original refresh plan.
    pub plan: SourceRefreshPlan,
    /// Actor who confirmed the refresh.
    pub confirmed_by: String,
}

/// Source citation suitable for Evidence Packet summaries.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceGraphEvidenceCitation {
    /// Source id.
    pub source_id: String,
    /// Source pin.
    pub pin: String,
    /// Graph paths touching this source for the WorkOrder.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub graph_paths: Vec<String>,
    /// Why this source was used.
    pub reason: String,
}

/// Save a Source Graph to `.coderoom/source-graph.toml`.
pub fn save_source_graph(project_root: &Path, graph: &SourceGraph) -> Result<PathBuf> {
    graph.validate()?;
    let path = source_graph_path(project_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let content = toml::to_string_pretty(graph).context("serializing Source Graph")?;
    std::fs::write(&path, ensure_trailing_newline(&content))
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

/// Load and validate a Source Graph from TOML.
pub fn load_source_graph(path: &Path) -> Result<SourceGraph> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let graph: SourceGraph =
        toml::from_str(&content).with_context(|| format!("parsing {}", path.display()))?;
    graph.validate()?;
    Ok(graph)
}

/// Return the canonical Source Graph path for a project root.
pub fn source_graph_path(project_root: &Path) -> PathBuf {
    project_root.join(CODEROOM_DIR).join(SOURCE_GRAPH_FILE)
}

fn role_set(roles: &[String]) -> BTreeSet<&str> {
    roles.iter().map(String::as_str).collect()
}

fn validate_roles(field: &str, roles: &[String]) -> Result<()> {
    if roles.is_empty() {
        bail!("{field} cannot be empty");
    }
    for role in roles {
        ensure_nonempty("role", role)?;
        if role.contains('/') || role.contains('\\') {
            bail!("role `{role}` must not contain path separators");
        }
    }
    Ok(())
}

fn validate_work_order_ref(work_order: &str) -> Result<()> {
    ensure_nonempty("workOrder", work_order)?;
    if !work_order.starts_with("WO-") || !work_order[3..].chars().all(|ch| ch.is_ascii_digit()) {
        bail!("workOrder `{work_order}` must use `WO-<digits>`");
    }
    Ok(())
}

fn ensure_source_id(field: &str, value: &str) -> Result<()> {
    ensure_nonempty(field, value)?;
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        Ok(())
    } else {
        bail!("{field} `{value}` may only use ASCII letters, digits, '-', '_', or '.'")
    }
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

const fn starts_with_const(input: &str, prefix: &str) -> bool {
    let input = input.as_bytes();
    let prefix = prefix.as_bytes();
    if prefix.len() > input.len() {
        return false;
    }
    let mut i = 0;
    while i < prefix.len() {
        if input[i] != prefix[i] {
            return false;
        }
        i += 1;
    }
    true
}

fn default_schema_version() -> u32 {
    SOURCE_GRAPH_SCHEMA_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(source_id: &str, kind: SourceGraphNodeKind, pin: &str) -> SourceGraphNode {
        SourceGraphNode {
            source_id: source_id.to_owned(),
            kind,
            pin: pin.to_owned(),
            trust_level: SourceTrustLevel::Internal,
            owner: "platform".to_owned(),
            visible_roles: vec!["host".to_owned(), "engineer".to_owned()],
            purpose: "Test source.".to_owned(),
            work_orders: vec!["WO-0216".to_owned()],
            path: Some(format!("sources/{source_id}")),
            url: None,
            snapshot_ref: None,
        }
    }

    #[test]
    fn graph_validates_edges_and_renders_work_order_sources() {
        let graph = SourceGraph {
            schema_version: SOURCE_GRAPH_SCHEMA_VERSION,
            nodes: vec![
                node("app", SourceGraphNodeKind::ProjectFile, "sha256:app"),
                node("core-api", SourceGraphNodeKind::LocalRepo, "commit:abc"),
            ],
            edges: vec![SourceGraphEdge {
                from: "app".to_owned(),
                to: "core-api".to_owned(),
                relation: SourceGraphRelation::DependsOn,
                reason: "App calls Core API.".to_owned(),
                owner: "platform".to_owned(),
                trust_level: SourceTrustLevel::Internal,
                work_orders: vec!["WO-0216".to_owned()],
            }],
        };

        graph.validate().expect("valid graph");
        let explanation = graph
            .explain_work_order_sources("WO-0216")
            .expect("explanation");

        assert!(explanation.contains("core-api"));
        assert!(explanation.contains("app -> core-api"));
    }

    #[test]
    fn refresh_plan_requires_confirmation_and_blocks_silent_refresh() {
        let plan = SourceRefreshPlan::new("core-api", "commit:abc", "commit:def").expect("plan");

        assert!(plan.requires_confirmation);
        assert!(!plan.allows_silent_refresh);

        let confirmed = plan.confirm("user").expect("confirmed");
        assert_eq!(confirmed.confirmed_by, "user");
    }
}
