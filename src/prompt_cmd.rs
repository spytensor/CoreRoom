//! Implementation of `cr prompt show`.

use std::path::Path;

use anyhow::{bail, Result};

use crate::config::{Config, COREROOM_DIR};
use crate::priors::{self, ComposeOptions};

/// Render the effective prompt for `role` exactly as the REPL would compose it.
pub fn render(project_root: &Path, role: &str) -> Result<String> {
    render_with_options(project_root, role, ComposeOptions::default())
}

/// Render the effective prompt for `role` with explicit composition options.
pub fn render_with_options(
    project_root: &Path,
    role: &str,
    options: ComposeOptions,
) -> Result<String> {
    let role = role.strip_prefix('@').unwrap_or(role);
    let cfg = load_config(project_root)?;
    if !cfg.roles.contains_key(role) {
        bail!("role `{role}` is not declared in .coreroom/config.toml");
    }
    priors::compose_for_with_options(&project_root.join(COREROOM_DIR), role, options)
}

#[cfg(not(test))]
fn load_config(project_root: &Path) -> crate::config::ConfigResult<Config> {
    Config::load(project_root)
}

#[cfg(test)]
fn load_config(project_root: &Path) -> crate::config::ConfigResult<Config> {
    Config::load_test(project_root)
}

/// Print the effective prompt for `role`.
pub fn show(project_root: &Path, role: &str) -> Result<()> {
    show_with_options(project_root, role, ComposeOptions::default())
}

/// Print the effective prompt for `role` with explicit composition options.
pub fn show_with_options(project_root: &Path, role: &str, options: ComposeOptions) -> Result<()> {
    print!("{}", render_with_options(project_root, role, options)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;
    use crate::config::{CONFIG_FILE, ROLES_DIR};

    fn fixture() -> TempDir {
        let tmp = TempDir::new().unwrap();
        let coreroom = tmp.path().join(COREROOM_DIR);
        fs::create_dir_all(coreroom.join(ROLES_DIR)).unwrap();
        fs::write(
            coreroom.join(CONFIG_FILE),
            r#"
default_engine = "cc"
permission_mode = "ask"
host_role = "host"

[roles.host]
[roles.backend]
"#,
        )
        .unwrap();
        for (role, body) in [("host", "HOST_PRIORS"), ("backend", "BACKEND_PRIORS")] {
            let role_dir = coreroom.join(ROLES_DIR).join(role);
            fs::create_dir_all(role_dir.join(crate::manifest::KNOWLEDGE_DIR)).unwrap();
            fs::write(role_dir.join(crate::manifest::ROLE_PRIORS_FILE), body).unwrap();
        }
        tmp
    }

    #[test]
    fn render_accepts_at_prefixed_role() {
        let tmp = fixture();
        let prompt = render(tmp.path(), "@backend").unwrap();
        assert!(prompt.contains("# CoreRoom kernel protocol"));
        assert!(prompt.contains("BACKEND_PRIORS"));
        assert!(prompt.contains("Source: .coreroom/roles/backend/priors.md"));
    }

    #[test]
    fn render_rejects_undeclared_role() {
        let tmp = fixture();
        let err = render(tmp.path(), "security").unwrap_err();
        assert!(
            format!("{err:#}").contains("role `security` is not declared"),
            "{err:#}"
        );
    }
}
