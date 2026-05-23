//! `cr role add/list/rm` implementations.
//!
//! Each command mutates `.coreroom/config.toml` and/or
//! `.coreroom/roles/<name>/priors.md` with the same validation discipline that
//! [`crate::config::Config::load`] enforces at REPL startup, so the
//! generated state is always loadable.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};

use crate::adapter::{Engine, PermissionMode};
use crate::config::{AuthorityScope, Config, RoleEntry, CONFIG_FILE, COREROOM_DIR, ROLES_DIR};
use crate::config_layered::ProjectConfigRaw;
use crate::{liveness, manifest};

/// Default body for a freshly-scaffolded role priors file. Users are
/// expected to replace this with project-specific guidance.
const DEFAULT_ROLE_PRIORS: &str = include_str!("init_defaults/role_template.md");

/// One role to append to an existing `.coreroom/` project config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RoleAddition {
    /// Role name without the leading `@`.
    pub(crate) name: String,
    /// Project-layer engine override. `None` inherits the effective
    /// default engine.
    pub(crate) engine: Option<Engine>,
    /// Project-layer model override. `None` inherits the effective
    /// model for that engine.
    pub(crate) model: Option<String>,
}

/// Add a new role. Updates `config.toml` (inserts `[roles.<name>]` with
/// optional engine/model overrides), then creates an empty priors file
/// at `.coreroom/roles/<name>/priors.md` if one doesn't already exist.
pub fn add(
    project_root: &Path,
    name: &str,
    engine: Option<Engine>,
    model: Option<&str>,
) -> Result<()> {
    validate_name(name)?;
    let coreroom_dir = project_root.join(COREROOM_DIR);
    if !coreroom_dir.is_dir() {
        bail!("{} not found — run `cr init` first", coreroom_dir.display());
    }

    let mut raw = read_project_raw(&coreroom_dir)?;
    if raw.roles.contains_key(name) {
        bail!("role `{name}` already exists in {CONFIG_FILE}");
    }

    let entry = RoleEntry {
        engine,
        model: model.map(ToOwned::to_owned),
        permission_mode: if matches!(engine, Some(Engine::Codex | Engine::Gemini)) {
            Some(PermissionMode::Bypass)
        } else {
            None
        },
        owner: None,
        authority: Vec::new(),
    };
    raw.roles.insert(name.to_owned(), entry);
    write_project_raw(&coreroom_dir, &raw)?;

    let priors_path = manifest::preferred_role_priors_path(&coreroom_dir, name);
    if priors_path.exists() {
        std::fs::create_dir_all(manifest::knowledge_dir(&coreroom_dir, name))
            .with_context(|| format!("creating knowledge dir for `{name}`"))?;
    } else if manifest::legacy_role_priors_path(&coreroom_dir, name).is_file() {
        manifest::ensure_role_dir_layout(&coreroom_dir, name)
            .with_context(|| format!("migrating legacy priors for `{name}`"))?;
    } else {
        let peers = role_peers(&raw, name);
        manifest::create_role_layout(
            &coreroom_dir,
            name,
            &render_role_template(name, &raw.host_role, &peers),
        )
        .with_context(|| format!("creating role layout for `{name}`"))?;
    }

    println!("✓ added role @{name}");
    if let Some(engine) = engine {
        println!("  engine: {}", engine.as_str());
    }
    if let Some(model) = model {
        println!("  model:  {model}");
    }
    println!("  priors: {}", priors_path.display());
    println!();
    println!("  edit the priors file, then `cr start` (or `/refresh @{name}` if running)");

    Ok(())
}

/// Attach a local markdown or text document to a role's knowledge mount.
pub fn attach(project_root: &Path, name: &str, source: &Path, alias: Option<&str>) -> Result<()> {
    let name = name.strip_prefix('@').unwrap_or(name);
    validate_name(name)?;
    let coreroom_dir = project_root.join(COREROOM_DIR);
    let raw = read_project_raw(&coreroom_dir)?;
    if !raw.roles.contains_key(name) {
        bail!("no such role: @{name}");
    }

    let outcome = manifest::attach_knowledge(&coreroom_dir, name, source, alias)?;
    if let Some(legacy) = outcome.migrated_legacy {
        println!(
            "migrated legacy priors {} -> {}",
            legacy.display(),
            manifest::preferred_role_priors_path(&coreroom_dir, name).display()
        );
    }
    println!("✓ attached {} to @{name}", outcome.entry.name);
    println!("  path:   {}", outcome.path.display());
    println!("  sha256: {}", outcome.entry.sha256);
    Ok(())
}

/// Detach a knowledge document from a role.
pub fn detach(project_root: &Path, name: &str, doc_name: &str) -> Result<()> {
    let name = name.strip_prefix('@').unwrap_or(name);
    validate_name(name)?;
    let coreroom_dir = project_root.join(COREROOM_DIR);
    let raw = read_project_raw(&coreroom_dir)?;
    if !raw.roles.contains_key(name) {
        bail!("no such role: @{name}");
    }

    let outcome = manifest::detach_knowledge(&coreroom_dir, name, doc_name)?;
    println!("✓ detached {} from @{name}", outcome.entry.name);
    if !outcome.removed_file {
        println!("  file was already missing: {}", outcome.path.display());
    }
    Ok(())
}

/// List a role's attached knowledge documents.
pub fn knowledge(project_root: &Path, name: &str, with_liveness: bool) -> Result<()> {
    let name = name.strip_prefix('@').unwrap_or(name);
    validate_name(name)?;
    let coreroom_dir = project_root.join(COREROOM_DIR);
    let raw = read_project_raw(&coreroom_dir)?;
    if !raw.roles.contains_key(name) {
        bail!("no such role: @{name}");
    }

    let entries = manifest::knowledge_inventory(&coreroom_dir, name)?;
    if entries.is_empty() {
        println!("(no knowledge attached for @{name})");
        return Ok(());
    }
    let liveness_doc = if with_liveness {
        Some(liveness::read(&coreroom_dir, name)?)
    } else {
        None
    };

    println!("@{name} knowledge:");
    if with_liveness {
        println!(
            "{:<28} {:>6} {:<20} {:<64} last-modified",
            "name", "hits", "last-loaded", "sha256"
        );
    } else {
        println!("{:<28} {:<64} last-modified", "name", "sha256");
    }
    for entry in entries {
        if let Some(doc) = &liveness_doc {
            let segment = doc
                .segments
                .get(&liveness::knowledge_segment_path(name, &entry.entry.name));
            let hits = segment.map_or(0, |segment| segment.hit_count);
            let last_loaded = segment
                .and_then(|segment| segment.last_matched_at.as_deref())
                .unwrap_or("-");
            println!(
                "{:<28} {:>6} {:<20} {:<64} {}",
                entry.entry.name,
                hits,
                truncate_timestamp(last_loaded),
                entry.entry.sha256,
                entry.modified_at.as_deref().unwrap_or("(missing)")
            );
        } else {
            println!(
                "{:<28} {:<64} {}",
                entry.entry.name,
                entry.entry.sha256,
                entry.modified_at.as_deref().unwrap_or("(missing)")
            );
        }
    }
    Ok(())
}

fn truncate_timestamp(value: &str) -> &str {
    value.get(..20).unwrap_or(value)
}

/// Add several roles in one config write. Priors files are created
/// before `config.toml` is updated, so a file-write failure cannot
/// leave config pointing at a missing role priors file.
pub(crate) fn add_many(project_root: &Path, additions: &[RoleAddition]) -> Result<usize> {
    let coreroom_dir = project_root.join(COREROOM_DIR);
    if !coreroom_dir.is_dir() {
        bail!("{} not found — run `cr init` first", coreroom_dir.display());
    }

    let raw = read_project_raw(&coreroom_dir)?;
    let mut to_add = Vec::new();
    for addition in additions {
        validate_name(&addition.name)?;
        if raw.roles.contains_key(&addition.name) {
            continue;
        }
        to_add.push(addition.clone());
    }
    if to_add.is_empty() {
        return Ok(0);
    }

    let updated_config = append_roles_config_body(&coreroom_dir, &to_add)?;

    let roles_dir = coreroom_dir.join(ROLES_DIR);
    std::fs::create_dir_all(&roles_dir)
        .with_context(|| format!("creating {}", roles_dir.display()))?;
    for addition in &to_add {
        let priors_path = manifest::preferred_role_priors_path(&coreroom_dir, &addition.name);
        if priors_path.exists() {
            std::fs::create_dir_all(manifest::knowledge_dir(&coreroom_dir, &addition.name))
                .with_context(|| format!("creating knowledge dir for `{}`", addition.name))?;
        } else if manifest::legacy_role_priors_path(&coreroom_dir, &addition.name).is_file() {
            manifest::ensure_role_dir_layout(&coreroom_dir, &addition.name)
                .with_context(|| format!("migrating legacy priors for `{}`", addition.name))?;
        } else {
            let mut peers = raw.roles.keys().cloned().collect::<Vec<_>>();
            peers.extend(
                to_add
                    .iter()
                    .map(|role| role.name.clone())
                    .filter(|name| name != &addition.name),
            );
            peers.sort();
            manifest::create_role_layout(
                &coreroom_dir,
                &addition.name,
                &render_role_template(&addition.name, &raw.host_role, &peers),
            )
            .with_context(|| format!("creating role layout for `{}`", addition.name))?;
        }
    }

    write_project_text(&coreroom_dir, &updated_config)?;

    Ok(to_add.len())
}

/// Print the configured roles, one per line, with engine + host marker.
///
/// Reads the merged `Config` (so the displayed engine/model reflects
/// the layered defaults), but never writes through it. Writes go via
/// [`read_project_raw`] / [`write_project_raw`] so user-level fields
/// don't accidentally end up in the committed project file.
pub fn list(project_root: &Path) -> Result<()> {
    let cfg = Config::load(project_root)?;

    let mut names: Vec<&str> = cfg.role_names().collect();
    names.sort_unstable();
    if names.is_empty() {
        println!("(no roles configured — use `cr role add <name>`)");
        return Ok(());
    }
    for name in names {
        let entry = cfg.roles.get(name);
        let engine = entry
            .and_then(|e| e.engine)
            .unwrap_or(cfg.default_engine)
            .as_str();
        let model = entry
            .and_then(|e| e.model.as_deref())
            .or(cfg.default_model.as_deref())
            .unwrap_or("(default)");
        let authority = entry
            .map(|e| authority_summary(&e.authority))
            .filter(|summary| !summary.is_empty())
            .map_or_else(String::new, |summary| format!(" authority=[{summary}]"));
        let host_marker = if cfg.is_host(name) { " (host)" } else { "" };
        println!("@{name:<14} engine={engine:<6} model={model}{authority}{host_marker}");
    }
    Ok(())
}

/// Print one role's effective identity surface.
pub fn show(project_root: &Path, name: &str) -> Result<()> {
    let cfg = Config::load(project_root)?;
    let coreroom_dir = project_root.join(COREROOM_DIR);
    let entry = cfg
        .roles
        .get(name)
        .with_context(|| format!("no such role: @{name}"))?;
    let role_cfg = cfg
        .role_config(name, &coreroom_dir)
        .with_context(|| format!("role `{name}` is declared but has invalid config"))?;

    println!("@{name}");
    println!(
        "  owner:           {}",
        entry.owner.as_deref().unwrap_or("(none)")
    );
    println!("  engine:          {}", role_cfg.engine.as_str());
    println!(
        "  model:           {}",
        role_cfg.model.as_deref().unwrap_or("(engine default)")
    );
    println!("  permission_mode: {}", role_cfg.permission_mode.as_str());
    println!(
        "  authority:       {}",
        if entry.authority.is_empty() {
            "(advisory only)".to_owned()
        } else {
            format!("[{}]", authority_summary(&entry.authority))
        }
    );
    Ok(())
}

/// Remove a role. Refuses if it's the configured host. Removes both the
/// `[roles.<name>]` table and the role priors/knowledge directory.
pub fn rm(project_root: &Path, name: &str) -> Result<()> {
    let coreroom_dir = project_root.join(COREROOM_DIR);
    let mut raw = read_project_raw(&coreroom_dir)?;

    if !raw.roles.contains_key(name) {
        bail!("no such role: @{name}");
    }
    if raw.host_role == name {
        bail!(
            "@{name} is the host role; change `host_role` in {CONFIG_FILE} first, \
             or use `cr role add` to introduce a replacement"
        );
    }

    raw.roles.remove(name);
    write_project_raw(&coreroom_dir, &raw)?;

    let legacy_path = manifest::legacy_role_priors_path(&coreroom_dir, name);
    if legacy_path.is_file() {
        std::fs::remove_file(&legacy_path)
            .with_context(|| format!("removing {}", legacy_path.display()))?;
    }
    let role_dir = manifest::role_dir(&coreroom_dir, name);
    if role_dir.is_dir() {
        std::fs::remove_dir_all(&role_dir)
            .with_context(|| format!("removing {}", role_dir.display()))?;
    }

    println!("✓ removed @{name}");
    Ok(())
}

/// Promote an existing role to be the project host. This persists the
/// `host_role` field in `.coreroom/config.toml`; the REPL `/host`
/// command is the session-only counterpart.
pub fn set_host(project_root: &Path, name: &str) -> Result<()> {
    validate_name(name)?;
    let coreroom_dir = project_root.join(COREROOM_DIR);
    let mut raw = read_project_raw(&coreroom_dir)?;

    if !raw.roles.contains_key(name) {
        bail!("no such role: @{name}");
    }
    name.clone_into(&mut raw.host_role);
    write_project_raw(&coreroom_dir, &raw)?;

    println!("✓ @{name} is now the host role");
    Ok(())
}

/// Set the human owner for an existing role.
pub fn set_owner(project_root: &Path, name: &str, owner: &str) -> Result<()> {
    if owner.trim().is_empty() {
        bail!("owner must be non-empty");
    }
    let coreroom_dir = project_root.join(COREROOM_DIR);
    let mut raw = read_project_raw(&coreroom_dir)?;
    let entry = raw
        .roles
        .get_mut(name)
        .with_context(|| format!("no such role: @{name}"))?;
    entry.owner = Some(owner.trim().to_owned());
    write_project_raw(&coreroom_dir, &raw)?;
    println!("✓ @{name} owner set to {}", owner.trim());
    Ok(())
}

/// Replace an existing role's authority scopes.
pub fn set_authority(project_root: &Path, name: &str, scopes: &[AuthorityScope]) -> Result<()> {
    let coreroom_dir = project_root.join(COREROOM_DIR);
    let mut raw = read_project_raw(&coreroom_dir)?;
    let entry = raw
        .roles
        .get_mut(name)
        .with_context(|| format!("no such role: @{name}"))?;
    let mut normalized = scopes.to_vec();
    normalized.sort();
    normalized.dedup();
    entry.authority = normalized;
    let summary = authority_summary(&entry.authority);
    write_project_raw(&coreroom_dir, &raw)?;
    println!("✓ @{name} authority set to [{summary}]");
    Ok(())
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("role name must be non-empty");
    }
    if name.starts_with('@') {
        bail!("role name should not include the leading `@`");
    }
    let allowed = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    if !name.chars().all(allowed) {
        bail!(
            "role name `{name}` contains invalid characters; use ASCII letters, digits, `-`, `_`"
        );
    }
    if !name.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        bail!("role name must start with an ASCII letter");
    }
    Ok(())
}

/// Read just the project-layer raw shape — never the merged config.
/// This keeps role-edit round-trips free of user-layer values (e.g.
/// the user's `default_engine` won't accidentally end up in the
/// committed project file when `cr role add` writes back).
fn read_project_raw(coreroom_dir: &Path) -> Result<ProjectConfigRaw> {
    let path = coreroom_dir.join(CONFIG_FILE);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let raw: ProjectConfigRaw =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(raw)
}

fn write_project_raw(coreroom_dir: &Path, raw: &ProjectConfigRaw) -> Result<()> {
    let body = toml::to_string_pretty(raw).map_err(|e| anyhow!("serializing config.toml: {e}"))?;
    write_project_text(coreroom_dir, &body)?;
    Ok(())
}

fn write_project_text(coreroom_dir: &Path, body: &str) -> Result<()> {
    let path = coreroom_dir.join(CONFIG_FILE);
    std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn append_roles_config_body(coreroom_dir: &Path, additions: &[RoleAddition]) -> Result<String> {
    let path = coreroom_dir.join(CONFIG_FILE);
    let mut body =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    if !body.ends_with('\n') {
        body.push('\n');
    }
    if !body.ends_with("\n\n") {
        body.push('\n');
    }

    for addition in additions {
        writeln!(&mut body, "[roles.{}]", addition.name)
            .expect("writing role table header to string should not fail");
        if let Some(engine) = addition.engine {
            writeln!(&mut body, "engine = \"{}\"", engine.as_str())
                .expect("writing role engine to string should not fail");
        }
        if let Some(model) = &addition.model {
            let value = toml::Value::String(model.clone());
            writeln!(&mut body, "model = {value}")
                .expect("writing role model to string should not fail");
        }
        if matches!(addition.engine, Some(Engine::Codex | Engine::Gemini)) {
            writeln!(&mut body, "permission_mode = \"bypass\"")
                .expect("writing role permission mode to string should not fail");
        }
        body.push('\n');
    }

    let _validated: ProjectConfigRaw =
        toml::from_str(&body).with_context(|| format!("validating {}", path.display()))?;
    Ok(body)
}

fn role_peers(raw: &ProjectConfigRaw, name: &str) -> Vec<String> {
    let mut peers = raw
        .roles
        .keys()
        .filter(|role| role.as_str() != name)
        .cloned()
        .collect::<Vec<_>>();
    peers.sort();
    peers
}

fn render_role_template(name: &str, host: &str, peers: &[String]) -> String {
    DEFAULT_ROLE_PRIORS
        .replace("{ROLE}", name)
        .replace("{HOST}", host)
        .replace(
            "{PEERS}",
            &if peers.is_empty() {
                "(none configured yet)".to_owned()
            } else {
                peers
                    .iter()
                    .map(|peer| format!("@{peer}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            },
        )
}

pub(crate) fn authority_summary(scopes: &[AuthorityScope]) -> String {
    scopes
        .iter()
        .map(|scope| scope.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests;
