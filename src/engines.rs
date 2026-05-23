//! Engine-binary detection and the no-engine abort screen.
//!
//! `cr` is a coordination shell: every role spawns one of `claude`,
//! `codex`, or `gemini`. If none of those CLIs are on `$PATH`, the rest
//! of `cr` cannot do useful work — `cr start` would fail at first spawn,
//! `cr init` would write a config that points at nothing.
//!
//! So we detect once at process entry and, if all three are missing,
//! print install instructions and exit. Subcommands that genuinely don't
//! need an engine (`cr config`, `cr update`) call this module's
//! `_optional` variant or simply skip the check.

use crate::adapter::Engine;
use crate::output;
use crossterm::style::Stylize;

/// Snapshot of which engine CLIs are installed on `$PATH` right now.
///
/// Not `Copy`: we pass it by reference to keep call sites consistent
/// with the rest of `init.rs` and to leave room for future fields
/// (paths, versions) without rewriting every signature.
#[derive(Debug, Clone)]
pub struct Engines {
    /// `claude` from `@anthropic-ai/claude-code`.
    pub cc: bool,
    /// `codex` from OpenAI Codex CLI.
    pub codex: bool,
    /// `gemini` from `@google/gemini-cli`.
    pub gemini: bool,
}

impl Engines {
    /// Probe `$PATH` for each engine binary. ~30 ms per probe; called
    /// once per `cr` invocation.
    #[must_use]
    pub fn detect() -> Self {
        Self {
            cc: bin_present("claude"),
            codex: bin_present("codex"),
            gemini: bin_present("gemini"),
        }
    }

    /// True when at least one of the three engines is on `$PATH`.
    #[must_use]
    pub fn any_installed(&self) -> bool {
        self.cc || self.codex || self.gemini
    }

    /// True when the named engine is on `$PATH`.
    #[must_use]
    pub fn is_present(&self, engine: Engine) -> bool {
        match engine {
            Engine::Cc => self.cc,
            Engine::Codex => self.codex,
            Engine::Gemini => self.gemini,
        }
    }
}

/// If no engine CLI is installed, print Screen 1 from `docs/colors.md`-era
/// design (install hints + abort) and return `Err(missing)`. Caller is
/// expected to bubble the error up so `main` exits non-zero.
///
/// Subcommands that work without an engine (`cr config`, `cr update`)
/// must NOT call this.
///
/// # Errors
/// Returns an opaque `anyhow::Error` whose message is what the user
/// already saw on stderr — useful for tests, ignored by `main`.
pub fn require_any_installed() -> anyhow::Result<Engines> {
    let engines = Engines::detect();
    if engines.any_installed() {
        return Ok(engines);
    }
    print_no_engine_screen();
    anyhow::bail!("no agent CLI installed");
}

/// The Screen 1 layout. Plain printing, no raw mode.
fn print_no_engine_screen() {
    println!();
    println!("{}", "CoreRoom · setup".with(output::EM).bold());
    println!("{}", "─────────────────".with(output::FADE));
    println!(
        "{}",
        "no agent CLI detected on this system.".with(output::TEXT)
    );
    println!();
    println!("{}", "CoreRoom needs at least one of:".with(output::TEXT));
    println!();
    // Pad the plain label first; styling SGR escapes would inflate the
    // byte length and break the column.
    for (label, cmd) in [
        ("claude", "npm install -g @anthropic-ai/claude-code"),
        (
            "codex",
            "brew install codex   (or: pipx install openai-codex)",
        ),
        ("gemini", "npm install -g @google/gemini-cli"),
    ] {
        let padded = format!("{label:<8}");
        println!(
            "  {}  {}",
            padded.with(output::KEY).bold(),
            cmd.with(output::TEXT),
        );
    }
    println!();
    println!("{}", "install one, then run `cr` again.".with(output::DIM));
    println!();
}

fn bin_present(name: &str) -> bool {
    use std::process::Command;
    Command::new(name)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_installed_true_when_any_flag_set() {
        let only_cc = Engines {
            cc: true,
            codex: false,
            gemini: false,
        };
        assert!(only_cc.any_installed());
        let only_gemini = Engines {
            cc: false,
            codex: false,
            gemini: true,
        };
        assert!(only_gemini.any_installed());
    }

    #[test]
    fn any_installed_false_when_all_absent() {
        let none = Engines {
            cc: false,
            codex: false,
            gemini: false,
        };
        assert!(!none.any_installed());
    }

    #[test]
    fn is_present_routes_to_the_right_flag() {
        let only_codex = Engines {
            cc: false,
            codex: true,
            gemini: false,
        };
        assert!(!only_codex.is_present(Engine::Cc));
        assert!(only_codex.is_present(Engine::Codex));
        assert!(!only_codex.is_present(Engine::Gemini));
    }
}
