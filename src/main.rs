//! `cr` — the CoreRoom CLI binary.
//!
//! Subcommands at v0.1:
//!
//! - `cr init [--project PATH]`  — bootstrap `.coreroom/` in a fresh project
//! - `cr role add <name> [--engine cc|codex|gemini] [--model X]` — add a role
//! - `cr role list`              — list configured roles
//! - `cr role show <name>`        — show role identity and authority
//! - `cr role attach <name> <file> [--name alias]` — mount role knowledge
//! - `cr role knowledge <name> [--with-liveness]` — list mounted role knowledge
//! - `cr role set-owner <name> <owner>` — set role owner
//! - `cr role set-authority <name> <scope...>` — set role authority
//! - `cr role rm <name>`         — remove a role (refuses for the host)
//! - `cr` — enter the console-first room, then continue into the REPL
//! - `cr start [--project PATH] [--allow-large-priors]` — enter the REPL directly
//! - `cr console [--project PATH] [--snapshot PATH]` — enter the v0.9 read-only full-screen console
//! - `cr prompt show <role>`     — print a role's effective prompt
//! - `cr lock`                   — regenerate `.coreroom/priors.lock`
//! - `cr verify`                 — verify priors lock content
//! - `cr gate ...`               — inspect SDLC gate ledgers
//! - `cr doctor [--fix] [--stale-days N]` — inspect CoreRoom project files
//! - `cr show [--role ROLE] [--since YYYY-MM-DD] [--tail N]` — replay events
//! - `cr cost [--since YYYY-MM-DD]` — summarize reported engine spend

use std::io::{IsTerminal, Write as _};
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use coreroom::adapter::{Engine, PermissionMode};
use coreroom::config::AuthorityScope;
use coreroom::config_cmd::LayerTarget;
use coreroom::crep::CrepEvent;
use coreroom::gate::{
    ArtifactInput, GateActor, GateArtifactKind, GateInit, GatePhase, GateTier, PhaseAdvanceInput,
    PlanOverrideInput, PlanReviewDecision, ReviewInput, RoleReviewInput, VerificationInput,
};
use coreroom::init::{InitHookMode, InitPreset};

#[derive(Debug, Parser)]
#[command(
    name = "cr",
    version,
    about = "CoreRoom — Engineering Control Room for AI Agents",
    long_about = None,
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Bootstrap a `.coreroom/` directory with detected default roles.
    Init {
        /// Project root in which to create `.coreroom/`. Defaults to the
        /// current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
        /// Skip the `proceed?` prompt and accept all defaults.
        /// For dotfile repos / onboarding scripts.
        #[arg(short = 'y', long = "yes")]
        yes: bool,
        /// Also scaffold Claude Code PreToolUse hooks under `.claude/`.
        #[arg(long)]
        with_claude_hooks: bool,
        /// Re-apply the latest Claude hook template to an existing project.
        #[arg(long)]
        upgrade_hooks: bool,
        /// Starter role preset: default or team.
        #[arg(long, value_parser = parse_init_preset, default_value = "default")]
        preset: InitPreset,
    },
    /// Regenerate `.coreroom/priors.lock` after intentional priors changes.
    Lock {
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// Verify `.coreroom/priors.lock` against current priors content.
    Verify {
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// Manage roles in the current project's `.coreroom/config.toml`.
    Role {
        #[command(subcommand)]
        command: RoleCmd,
    },
    /// Enter the interactive REPL using `.coreroom/config.toml` in the
    /// current directory (or `--project`).
    Start {
        /// Project root containing `.coreroom/`. Defaults to the current
        /// working directory.
        #[arg(long)]
        project: Option<PathBuf>,
        /// Run this session with permission_mode=bypass for every role.
        #[arg(long)]
        yolo: bool,
        /// Start every role with a fresh engine session instead of
        /// resuming the prior conversation. Default behaviour (per
        /// amendment A-006) is to resume from
        /// `.coreroom/sessions/ids/<role>.id` when present; pass
        /// `--fresh` to clear those ids and start clean. The user-
        /// facing equivalent of `claude --resume` vs no flag.
        #[arg(long)]
        fresh: bool,
        /// Allow composed role priors above the 500KB hard limit.
        #[arg(long)]
        allow_large_priors: bool,
    },
    /// Enter the v0.9 read-only full-screen console.
    Console {
        /// Project root containing `.coreroom/`. Defaults to the current
        /// working directory when `--snapshot` is not supplied.
        #[arg(long)]
        project: Option<PathBuf>,
        /// TOML CoreRoomSnapshot file to render. When omitted, CoreRoom
        /// derives a live local snapshot from project config and git state.
        #[arg(long)]
        snapshot: Option<PathBuf>,
    },
    /// Replay `.coreroom/messages.jsonl` through the live renderer.
    Show {
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
        /// Only replay events for this role. A leading `@` is accepted.
        #[arg(long)]
        role: Option<String>,
        /// Skip the log entirely if its mtime is older than this date
        /// (`YYYY-MM-DD`). v0.1 limitation — proper per-event timestamps
        /// land in v0.2.
        #[arg(long, value_parser = parse_date)]
        since: Option<chrono::NaiveDate>,
        /// Render only the last N matching events.
        #[arg(long)]
        tail: Option<usize>,
    },
    /// Per-role cost summary aggregated from `.coreroom/messages.jsonl`.
    Cost {
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
        /// Skip the log entirely if its mtime is older than this date
        /// (`YYYY-MM-DD`). v0.1 limitation — proper per-event timestamps
        /// land in v0.2.
        #[arg(long, value_parser = parse_date)]
        since: Option<chrono::NaiveDate>,
    },
    /// Compact archived patches and old journals into a role's priors.
    Compact {
        /// Role name to compact.
        role: String,
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// List every git-backed pointer the role's priors reference, with
    /// resolution status (fresh / stale / unresolvable).
    #[command(long_about = "\
List every `[[…]]` pointer in the role's priors file with its current \
resolution status. Useful for spotting which anchors fell behind HEAD \
before re-prompting.

Token grammar (write these inside a role's priors.md file):

  [[<path>#L<n>-<m>@<sha>]]   locked to a commit, line range
  [[<path>#L<n>@<sha>]]        locked single line
  [[<path>@<sha>]]              locked whole file
  [[<path>#L<n>-<m>]]           HEAD range
  [[<path>@HEAD]]                HEAD whole file (explicit)

Every pointer must carry at least one anchor — `#L<range>` or `@<sha>` / \
`@HEAD`. Unanchored `[[bare-word]]` tokens are intentionally rejected at \
parse time so prose like `[[TODO]]` doesn't accidentally trigger a file \
read. The HEAD-tracking branch is also containment-checked: any path \
that canonicalises outside the repo root is refused.

When a pointer's locked sha falls behind HEAD, the priors render flags it \
as stale and keeps the content from the original sha. Update the sha or \
switch to `@HEAD` when you've reviewed the new content.")]
    Pointers {
        /// Role name. Leading `@` is accepted.
        role: String,
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// Inspect or edit the layered config (user / project / .local).
    Config {
        #[command(subcommand)]
        command: ConfigCmd,
    },
    /// Inspect composed role prompts.
    Prompt {
        #[command(subcommand)]
        command: PromptCmd,
    },
    /// Inspect and update SDLC gate ledgers.
    Gate {
        #[command(subcommand)]
        command: GateCmd,
    },
    /// Diagnose CoreRoom project files.
    Doctor {
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
        /// Apply exact safe fixes.
        #[arg(long)]
        fix: bool,
        /// Priors liveness stale threshold.
        #[arg(long, default_value_t = coreroom::liveness::DEFAULT_STALE_DAYS)]
        stale_days: i64,
    },
    /// Check the npm registry for a newer `cr` and report the diff.
    /// Read-only — does not touch the installed binary. Run
    /// `cr upgrade` to actually install.
    Update,
    /// Upgrade the `cr` binary in place via whichever method
    /// originally installed it (currently only `npm install -g` is
    /// auto-upgradable; other paths print instructions). Verifies
    /// the binary on disk actually changed before claiming success.
    Upgrade,
    /// Internal Claude Code hook entry point.
    #[command(name = "__coreroom-hook-decision", hide = true)]
    HookDecision {
        /// Permission mode to apply to this hook decision.
        #[arg(long, value_parser = parse_permission_mode)]
        mode: PermissionMode,
        /// Session policy file populated by `/allow` and `/deny`.
        #[arg(long)]
        policy_file: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum RoleCmd {
    /// Add a new role.
    Add {
        /// Role name (ASCII letters/digits/`-`/`_`, must start with a letter).
        name: String,
        /// Override default engine for this role.
        #[arg(long, value_parser = parse_engine)]
        engine: Option<Engine>,
        /// Override default model for this role.
        #[arg(long)]
        model: Option<String>,
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// List configured roles.
    List {
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// Show a role's effective identity and authority.
    Show {
        /// Role name to inspect.
        name: String,
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// Attach a markdown or text file to a role's knowledge mount.
    Attach {
        /// Role name to update.
        role: String,
        /// Local markdown or text file to copy into role knowledge.
        file_path: PathBuf,
        /// Alias filename to use under knowledge/.
        #[arg(long = "name")]
        alias: Option<String>,
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// Detach a knowledge file from a role.
    Detach {
        /// Role name to update.
        role: String,
        /// Knowledge filename to remove.
        name: String,
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// List a role's mounted knowledge files.
    Knowledge {
        /// Role name to inspect.
        role: String,
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
        /// Include local liveness telemetry for each mounted file.
        #[arg(long)]
        with_liveness: bool,
    },
    /// Remove a role (refuses for the configured host).
    Rm {
        /// Role name to remove.
        name: String,
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// Promote an existing role to host in project config.
    Host {
        /// Role name to make host.
        name: String,
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// Set the human owner for a role.
    SetOwner {
        /// Role name to update.
        name: String,
        /// Owner email or handle.
        owner: String,
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// Replace the authority scopes for a role.
    SetAuthority {
        /// Role name to update.
        name: String,
        /// Canonical scopes such as deployment, infra, secrets.
        #[arg(value_parser = parse_authority_scope, num_args = 1..)]
        scopes: Vec<AuthorityScope>,
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigCmd {
    /// Print the effective merged config plus which layer files were
    /// read. Use this to debug "why is my engine cc when I set codex
    /// in user config?" — answer is in the layer footer.
    Show {
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// Open `$EDITOR` (or `$VISUAL`) on a layer's config file.
    /// Creates a commented stub for `--user` / `--local` if missing;
    /// refuses `--project` if `.coreroom/config.toml` is missing
    /// (run `cr init` first).
    Edit {
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
        /// Edit the user-level config (~/.config/coreroom/config.toml).
        #[arg(long, group = "layer")]
        user: bool,
        /// Edit the project-local override (.coreroom/config.local.toml).
        #[arg(long, group = "layer")]
        local: bool,
    },
    /// Print the absolute path of a layer's config file.
    Path {
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
        /// Print the user-level path.
        #[arg(long, group = "layer")]
        user: bool,
        /// Print the project-local path.
        #[arg(long, group = "layer")]
        local: bool,
    },
    /// Print one effective config value, such as `default_engine`.
    Get {
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
        /// Dotted key to read.
        key: String,
    },
    /// Set one config value. Defaults to the user layer; pass
    /// `--project-layer` or `--local` for other writable layers.
    Set {
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
        /// Write project `.coreroom/config.toml`.
        #[arg(long = "project-layer", group = "layer")]
        project_layer: bool,
        /// Write the project-local override.
        #[arg(long, group = "layer")]
        local: bool,
        /// Dotted key to set.
        key: String,
        /// TOML-ish scalar value to write.
        value: String,
    },
}

#[derive(Debug, Subcommand)]
enum PromptCmd {
    /// Print the effective prompt for one role.
    Show {
        /// Role name. A leading `@` is accepted.
        role: String,
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
        /// Allow composed role priors above the 500KB hard limit.
        #[arg(long)]
        allow_large_priors: bool,
    },
}

#[derive(Debug, Subcommand)]
enum GateCmd {
    /// Create or replace a per-thread SDLC gate ledger.
    Init {
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
        /// CoreRoom thread id.
        #[arg(long)]
        thread: String,
        /// Gate tier: 0 or 1.
        #[arg(long, value_parser = parse_gate_tier)]
        tier: GateTier,
        /// Work item title.
        #[arg(long)]
        feature: String,
        /// Initial phase. Defaults to intake.
        #[arg(long, value_parser = parse_gate_phase, default_value = "intake")]
        phase: GatePhase,
        /// Implementing role, when known.
        #[arg(long)]
        role: Option<String>,
        /// Implementing engine, when known.
        #[arg(long, value_parser = parse_engine)]
        engine: Option<Engine>,
        /// Implementing model identifier, when known.
        #[arg(long)]
        model: Option<String>,
        /// Implementing turn id, when known.
        #[arg(long)]
        turn: Option<String>,
    },
    /// Print the selected gate status.
    Status {
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
        /// Thread id. Defaults to the active gate pointer.
        #[arg(long)]
        thread: Option<String>,
    },
    /// Validate the selected gate structurally.
    Validate {
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
        /// Thread id. Defaults to the active gate pointer.
        #[arg(long)]
        thread: Option<String>,
    },
    /// Explicitly advance or roll back a gate phase.
    Phase {
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
        /// CoreRoom thread id.
        thread: String,
        /// Target phase.
        #[arg(value_parser = parse_gate_phase)]
        next_phase: GatePhase,
        /// Actor responsible for this transition. Defaults to `user`.
        #[arg(long, default_value = "user")]
        actor: String,
        /// Roll back to an earlier phase with this justification.
        #[arg(long)]
        rollback: Option<String>,
    },
    /// Close a gate only when validation passes, unless bypassed.
    Close {
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
        /// Thread id. Defaults to the active gate pointer.
        #[arg(long)]
        thread: Option<String>,
        /// Explicit human-readable bypass reason.
        #[arg(long = "bypass")]
        bypass_reason: Option<String>,
    },
    /// Record an explicit bypass or accepted-risk reason.
    Bypass {
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
        /// Thread id. Defaults to the active gate pointer.
        #[arg(long)]
        thread: Option<String>,
        /// Gate or rule being bypassed.
        #[arg(long, default_value = "manual")]
        gate: String,
        /// Human-readable reason.
        #[arg(long)]
        reason: String,
    },
    /// Record an evidence artifact.
    Artifact {
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
        /// Thread id. Defaults to the active gate pointer.
        #[arg(long)]
        thread: Option<String>,
        /// Artifact kind.
        #[arg(long, value_parser = parse_artifact_kind)]
        kind: GateArtifactKind,
        /// Artifact path.
        #[arg(long)]
        path: String,
        /// Producing role.
        #[arg(long)]
        role: Option<String>,
        /// Producing turn id.
        #[arg(long)]
        turn: Option<String>,
    },
    /// Record implementer metadata.
    Implementer {
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
        /// Thread id. Defaults to the active gate pointer.
        #[arg(long)]
        thread: Option<String>,
        /// Implementing role.
        #[arg(long)]
        role: String,
        /// Implementing engine.
        #[arg(long, value_parser = parse_engine)]
        engine: Engine,
        /// Implementing model identifier.
        #[arg(long)]
        model: String,
        /// Implementing turn id, when known.
        #[arg(long)]
        turn: Option<String>,
    },
    /// Record a review turn.
    Reviewer {
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
        /// Thread id. Defaults to the active gate pointer.
        #[arg(long)]
        thread: Option<String>,
        /// Reviewer role.
        #[arg(long)]
        role: String,
        /// Reviewer engine.
        #[arg(long, value_parser = parse_engine)]
        engine: Engine,
        /// Reviewer model identifier.
        #[arg(long)]
        model: String,
        /// Reviewer turn id, when known.
        #[arg(long)]
        turn: Option<String>,
        /// Review artifact path.
        #[arg(long)]
        artifact: Option<String>,
        /// Mark this review as same-role/self review.
        #[arg(long)]
        same_role: bool,
        /// Blocking finding count.
        #[arg(long, default_value_t = 0)]
        blocking_count: u32,
        /// Warning finding count.
        #[arg(long, default_value_t = 0)]
        warning_count: u32,
        /// Whether review includes file:line evidence.
        #[arg(long)]
        file_line_evidence: bool,
        /// Whether all blocking findings are resolved.
        #[arg(long)]
        all_blockings_resolved: bool,
    },
    /// Record an authority-scoped plan review decision.
    RoleReview {
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
        /// CoreRoom thread id.
        thread: String,
        /// Reviewer role. A leading `@` is accepted.
        role: String,
        /// Review decision: approve, reject, or needs-revision.
        #[arg(value_parser = parse_plan_review_decision)]
        decision: PlanReviewDecision,
        /// Human-readable reason. Required for reject and needs-revision.
        #[arg(long)]
        reason: Option<String>,
    },
    /// Override a blocking authority-scoped plan review decision.
    Override {
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
        /// CoreRoom thread id.
        thread: String,
        /// Role whose blocking review is overruled.
        #[arg(long)]
        role: String,
        /// Human-readable override reason.
        #[arg(long)]
        reason: String,
    },
    /// Record verification evidence.
    Verify {
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
        /// Thread id. Defaults to the active gate pointer.
        #[arg(long)]
        thread: Option<String>,
        /// Command or verification method.
        #[arg(long)]
        command: String,
        /// Evidence text or command output.
        #[arg(long)]
        evidence: String,
        /// Whether verification passed.
        #[arg(long)]
        ok: bool,
    },
    /// Print the selected raw ledger JSON.
    Show {
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
        /// Thread id. Defaults to the active gate pointer.
        #[arg(long)]
        thread: Option<String>,
    },
    /// Install missing SDLC gate templates into `.coreroom/`.
    Templates {
        /// Project root. Defaults to the current working directory.
        #[arg(long)]
        project: Option<PathBuf>,
        /// Overwrite existing template files.
        #[arg(long)]
        overwrite: bool,
    },
}

fn layer_from_flags(user: bool, local: bool) -> LayerTarget {
    match (user, local) {
        (true, _) => LayerTarget::User,
        (_, true) => LayerTarget::Local,
        // Default: project. clap's `group = "layer"` already makes
        // --user / --local mutually exclusive at parse time.
        _ => LayerTarget::Project,
    }
}

fn parse_engine(s: &str) -> Result<Engine, String> {
    match s {
        "cc" => Ok(Engine::Cc),
        "codex" => Ok(Engine::Codex),
        "gemini" => Ok(Engine::Gemini),
        other => Err(format!(
            "unknown engine `{other}` — valid: cc, codex, gemini"
        )),
    }
}

fn parse_permission_mode(s: &str) -> Result<PermissionMode, String> {
    match s {
        "ask" => Ok(PermissionMode::Ask),
        "auto" => Ok(PermissionMode::Auto),
        "bypass" => Ok(PermissionMode::Bypass),
        other => Err(format!(
            "unknown permission mode `{other}` — valid: ask, auto, bypass"
        )),
    }
}

fn parse_authority_scope(s: &str) -> Result<AuthorityScope, String> {
    AuthorityScope::parse(s).ok_or_else(|| {
        format!(
            "unknown authority scope `{s}`; expected one of: {}",
            AuthorityScope::expected_values()
        )
    })
}

fn parse_gate_tier(s: &str) -> Result<GateTier, String> {
    GateTier::parse(s).map_err(|error| error.to_string())
}

fn parse_gate_phase(s: &str) -> Result<GatePhase, String> {
    GatePhase::parse(s).map_err(|error| error.to_string())
}

fn parse_artifact_kind(s: &str) -> Result<GateArtifactKind, String> {
    GateArtifactKind::parse(s).map_err(|error| error.to_string())
}

fn parse_plan_review_decision(s: &str) -> Result<PlanReviewDecision, String> {
    PlanReviewDecision::parse(s).map_err(|error| error.to_string())
}

fn parse_init_preset(s: &str) -> Result<InitPreset, String> {
    InitPreset::parse(s).map_err(|error| error.to_string())
}

fn parse_date(s: &str) -> std::result::Result<chrono::NaiveDate, String> {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|e| format!("must be YYYY-MM-DD: {e}"))
}

fn project_root_or_cwd(arg: Option<PathBuf>) -> std::io::Result<PathBuf> {
    match arg {
        Some(p) => Ok(p),
        None => std::env::current_dir(),
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(Cmd::HookDecision { mode, policy_file }) = &cli.command {
        return coreroom::permissions::run_claude_hook(*mode, policy_file.as_deref());
    }

    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .try_init();
    coreroom::output::print_terminal_probe();

    // Engine-binary check up front. `cr config`, `cr update`, and
    // `cr upgrade` are useful without any engine installed (inspecting
    // or fixing the very setup that's missing); everything else
    // requires at least one of claude / codex / gemini on $PATH.
    let needs_engine = !matches!(
        cli.command,
        Some(
            Cmd::Init { .. }
                | Cmd::Lock { .. }
                | Cmd::Verify { .. }
                | Cmd::Role { .. }
                | Cmd::Config { .. }
                | Cmd::Prompt { .. }
                | Cmd::Gate { .. }
                | Cmd::Console { .. }
                | Cmd::Doctor { .. }
                | Cmd::Update
                | Cmd::Upgrade
                | Cmd::HookDecision { .. }
        )
    );
    if needs_engine && coreroom::engines::require_any_installed().is_err() {
        std::process::exit(1);
    }

    match cli.command {
        None => run_console_first_default(),
        Some(Cmd::Init {
            project,
            yes,
            with_claude_hooks,
            upgrade_hooks,
            preset,
        }) => {
            let mut opts = if yes {
                coreroom::init::InitOptions::accepted_defaults()
            } else {
                coreroom::init::InitOptions::manual()
            };
            opts.hook_mode = if with_claude_hooks {
                InitHookMode::InstallOrUpgrade
            } else if upgrade_hooks {
                InitHookMode::UpgradeExisting
            } else {
                InitHookMode::None
            };
            opts.preset = preset;
            coreroom::init::run(&project_root_or_cwd(project)?, opts)
        }
        Some(Cmd::Lock { project }) => run_lock(project),
        Some(Cmd::Verify { project }) => run_verify(project),
        Some(Cmd::Role { command }) => run_role_cmd(command),
        Some(Cmd::Start {
            project,
            yolo,
            fresh,
            allow_large_priors,
        }) => run_start(project, yolo, fresh, allow_large_priors),
        Some(Cmd::Console { project, snapshot }) => run_console(project, snapshot),
        Some(Cmd::Show {
            project,
            role,
            since,
            tail,
        }) => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            runtime.block_on(async move {
                let project_root = project_root_or_cwd(project)?;
                let options = coreroom::repl::ShowOptions {
                    role: role.map(|role| role.strip_prefix('@').unwrap_or(&role).to_owned()),
                    since,
                    tail,
                };
                coreroom::repl::show_log(&project_root, &options).await
            })
        }
        Some(Cmd::Config { command }) => run_config_cmd(command),
        Some(Cmd::Prompt { command }) => run_prompt_cmd(command),
        Some(Cmd::Gate { command }) => run_gate_cmd(command),
        Some(Cmd::Doctor {
            project,
            fix,
            stale_days,
        }) => {
            let root = project_root_or_cwd(project)?;
            coreroom::doctor::run(&root, coreroom::doctor::DoctorOptions { fix, stale_days })
        }
        Some(Cmd::Update) => coreroom::update::check(),
        Some(Cmd::Upgrade) => coreroom::update::upgrade(),
        Some(Cmd::HookDecision { .. }) => unreachable!("handled before terminal setup"),
        Some(Cmd::Compact { role, project }) => {
            let root = project_root_or_cwd(project)?;
            let role = role.strip_prefix('@').unwrap_or(&role);
            let path =
                coreroom::priors::compact_role(&root.join(coreroom::config::COREROOM_DIR), role)?;
            println!("compacted @{role} history into {}", path.display());
            Ok(())
        }
        Some(Cmd::Pointers { role, project }) => {
            let root = project_root_or_cwd(project)?;
            let role = role.strip_prefix('@').unwrap_or(&role);
            run_pointers(&root, role)
        }
        Some(Cmd::Cost { project, since }) => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            runtime.block_on(async move {
                let project_root = project_root_or_cwd(project)?;
                coreroom::cost::run(&project_root, since).await
            })
        }
    }
}

fn run_prompt_cmd(cmd: PromptCmd) -> Result<()> {
    match cmd {
        PromptCmd::Show {
            role,
            project,
            allow_large_priors,
        } => coreroom::prompt_cmd::show_with_options(
            &project_root_or_cwd(project)?,
            &role,
            coreroom::priors::ComposeOptions {
                allow_large_priors,
                ..Default::default()
            },
        ),
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "gate subcommand plumbing is a flat CLI dispatcher"
)]
fn run_gate_cmd(cmd: GateCmd) -> Result<()> {
    match cmd {
        GateCmd::Init {
            project,
            thread,
            tier,
            feature,
            phase,
            role,
            engine,
            model,
            turn,
        } => {
            let root = project_root_or_cwd(project)?;
            let implementer = match (role, engine, model) {
                (Some(role), Some(engine), Some(model)) => Some(GateActor {
                    role: normalize_role(&role),
                    engine,
                    model,
                    turn_id: turn,
                    thread_id: Some(thread.clone()),
                }),
                (None, None, None) => None,
                _ => anyhow::bail!(
                    "--role, --engine, and --model must be supplied together for implementer metadata"
                ),
            };
            let ledger = coreroom::gate::init(
                &root,
                GateInit {
                    thread_id: thread,
                    feature,
                    tier,
                    phase,
                    implementer,
                },
            )?;
            println!(
                "created {} gate for `{}` ({})",
                ledger.tier.label(),
                ledger.feature,
                ledger.thread_id
            );
            Ok(())
        }
        GateCmd::Status { project, thread } => {
            let root = project_root_or_cwd(project)?;
            let ledger = coreroom::gate::load(&root, thread.as_deref())?;
            let validation = coreroom::gate::validate(&root, Some(&ledger.thread_id))?;
            println!("{}", coreroom::gate::format_status(&ledger, &validation));
            Ok(())
        }
        GateCmd::Validate { project, thread } => {
            let root = project_root_or_cwd(project)?;
            let validation = coreroom::gate::validate(&root, thread.as_deref())?;
            if validation.passed() {
                println!(
                    "{} gate pass ({})",
                    validation.tier.label(),
                    validation.thread_id
                );
            } else {
                println!("{}", coreroom::gate::format_blocking_message(&validation));
            }
            Ok(())
        }
        GateCmd::Phase {
            project,
            thread,
            next_phase,
            actor,
            rollback,
        } => {
            let root = project_root_or_cwd(project)?;
            let transition = coreroom::gate::advance_phase(
                &root,
                &PhaseAdvanceInput {
                    thread_id: thread,
                    to: next_phase,
                    actor,
                    rollback_reason: rollback,
                },
            )?;
            append_crep_event(
                &root,
                &CrepEvent::PhaseAdvanced {
                    thread: transition.thread_id.clone(),
                    priors_hash: String::new(),
                    from: transition.from,
                    to: transition.to,
                    actor: transition.actor.clone(),
                },
            )?;
            if let Some(reason) = transition.rollback_reason {
                println!(
                    "rolled back {}: {} -> {} ({reason})",
                    transition.thread_id,
                    transition.from.label(),
                    transition.to.label()
                );
            } else {
                println!(
                    "advanced {}: {} -> {}",
                    transition.thread_id,
                    transition.from.label(),
                    transition.to.label()
                );
            }
            Ok(())
        }
        GateCmd::Close {
            project,
            thread,
            bypass_reason,
        } => {
            let root = project_root_or_cwd(project)?;
            let thread = selected_gate_thread(&root, thread.as_deref())?;
            let ledger = coreroom::gate::close(&root, &thread, bypass_reason.as_deref())?;
            println!(
                "closed gate {} with result {}",
                ledger.thread_id,
                ledger.result.label()
            );
            Ok(())
        }
        GateCmd::Bypass {
            project,
            thread,
            gate,
            reason,
        } => {
            let root = project_root_or_cwd(project)?;
            let thread = selected_gate_thread(&root, thread.as_deref())?;
            let ledger = coreroom::gate::record_bypass(&root, &thread, &gate, &reason)?;
            println!(
                "recorded bypass for {} ({})",
                ledger.thread_id,
                ledger.result.label()
            );
            Ok(())
        }
        GateCmd::Artifact {
            project,
            thread,
            kind,
            path,
            role,
            turn,
        } => {
            let root = project_root_or_cwd(project)?;
            let thread = selected_gate_thread(&root, thread.as_deref())?;
            let ledger = coreroom::gate::record_artifact(
                &root,
                ArtifactInput {
                    thread_id: thread,
                    kind,
                    path,
                    role: role.map(|role| normalize_role(&role)),
                    turn_id: turn,
                },
            )?;
            println!(
                "recorded {} artifact for {}",
                kind.label(),
                ledger.thread_id
            );
            Ok(())
        }
        GateCmd::Implementer {
            project,
            thread,
            role,
            engine,
            model,
            turn,
        } => {
            let root = project_root_or_cwd(project)?;
            let thread = selected_gate_thread(&root, thread.as_deref())?;
            let actor = GateActor {
                role: normalize_role(&role),
                engine,
                model,
                turn_id: turn,
                thread_id: Some(thread.clone()),
            };
            let ledger = coreroom::gate::set_implementer(&root, &thread, actor)?;
            println!("recorded implementer for {}", ledger.thread_id);
            Ok(())
        }
        GateCmd::Reviewer {
            project,
            thread,
            role,
            engine,
            model,
            turn,
            artifact,
            same_role,
            blocking_count,
            warning_count,
            file_line_evidence,
            all_blockings_resolved,
        } => {
            let root = project_root_or_cwd(project)?;
            let thread = selected_gate_thread(&root, thread.as_deref())?;
            let reviewer = GateActor {
                role: normalize_role(&role),
                engine,
                model,
                turn_id: turn,
                thread_id: Some(thread.clone()),
            };
            let ledger = coreroom::gate::record_review(
                &root,
                ReviewInput {
                    thread_id: thread,
                    reviewer,
                    same_role_as_implementer: same_role,
                    blocking_count,
                    warning_count,
                    file_line_evidence,
                    all_blockings_resolved,
                    artifact_path: artifact,
                },
            )?;
            println!("recorded reviewer for {}", ledger.thread_id);
            Ok(())
        }
        GateCmd::RoleReview {
            project,
            thread,
            role,
            decision,
            reason,
        } => {
            let root = project_root_or_cwd(project)?;
            let record = coreroom::gate::record_role_review(
                &root,
                RoleReviewInput {
                    thread_id: thread.clone(),
                    role,
                    decision,
                    reason,
                },
            )?;
            append_crep_event(
                &root,
                &CrepEvent::PlanReviewed {
                    role: record.reviewer.role.clone(),
                    priors_hash: String::new(),
                    decision: record.decision,
                    plan_sha: record.plan_sha.clone(),
                },
            )?;
            println!(
                "recorded {} review from @{} for {} ({})",
                record.decision.label(),
                record.reviewer.role,
                thread,
                short_sha(&record.plan_sha)
            );
            Ok(())
        }
        GateCmd::Override {
            project,
            thread,
            role,
            reason,
        } => {
            let root = project_root_or_cwd(project)?;
            let override_record = coreroom::gate::record_plan_override(
                &root,
                PlanOverrideInput {
                    thread_id: thread.clone(),
                    role,
                    reason,
                },
            )?;
            append_crep_event(
                &root,
                &CrepEvent::PlanOverridden {
                    role: override_record.role.clone(),
                    priors_hash: String::new(),
                    reason: override_record.reason.clone(),
                },
            )?;
            println!(
                "overrode @{} plan review for {} ({})",
                override_record.role,
                thread,
                short_sha(&override_record.plan_sha)
            );
            Ok(())
        }
        GateCmd::Verify {
            project,
            thread,
            command,
            evidence,
            ok,
        } => {
            let root = project_root_or_cwd(project)?;
            let thread = selected_gate_thread(&root, thread.as_deref())?;
            let ledger = coreroom::gate::record_verification(
                &root,
                VerificationInput {
                    thread_id: thread,
                    command,
                    ok,
                    evidence,
                },
            )?;
            println!("recorded verification for {}", ledger.thread_id);
            Ok(())
        }
        GateCmd::Show { project, thread } => {
            let root = project_root_or_cwd(project)?;
            let ledger = coreroom::gate::load(&root, thread.as_deref())?;
            println!("{}", serde_json::to_string_pretty(&ledger)?);
            Ok(())
        }
        GateCmd::Templates { project, overwrite } => {
            let root = project_root_or_cwd(project)?;
            let coreroom_dir = root.join(coreroom::config::COREROOM_DIR);
            if !coreroom_dir.exists() {
                anyhow::bail!("{} is missing; run `cr init` first", coreroom_dir.display());
            }
            let outcome = coreroom::gate::install_templates(&coreroom_dir, overwrite)?;
            println!(
                "gate templates: {} written, {} skipped",
                outcome.written, outcome.skipped
            );
            Ok(())
        }
    }
}

fn selected_gate_thread(root: &Path, explicit: Option<&str>) -> Result<String> {
    Ok(coreroom::gate::load(root, explicit)?.thread_id)
}

fn append_crep_event(root: &Path, event: &CrepEvent) -> Result<()> {
    use std::io::Write as _;

    let coreroom_dir = root.join(coreroom::config::COREROOM_DIR);
    std::fs::create_dir_all(&coreroom_dir)?;
    let path = coreroom_dir.join("messages.jsonl");
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&path)?;
    writeln!(file, "{}", serde_json::to_string(event)?)?;
    Ok(())
}

fn short_sha(sha: &str) -> &str {
    sha.get(..12).unwrap_or(sha)
}

fn normalize_role(role: &str) -> String {
    role.strip_prefix('@').unwrap_or(role).to_owned()
}

/// `cr pointers @<role>` — read the role's priors file and list each
/// `[[…]]` pointer with its resolution status. Lives here (not in the
/// `pointers` library module) so the module stays pure data-in /
/// data-out with no `crate::config` or `println!` dependencies; the
/// future Contracts / Inbox layers reuse the library half.
fn run_pointers(project_root: &Path, role: &str) -> Result<()> {
    use coreroom::config::COREROOM_DIR;
    use coreroom::pointers::{
        resolve_all, status_glyph, status_word, Pointer, PointerStatus, UnresolvableReason,
    };

    let coreroom_dir = project_root.join(COREROOM_DIR);
    let priors_path = coreroom::manifest::role_priors_path_existing(&coreroom_dir, role)
        .unwrap_or_else(|| coreroom::manifest::preferred_role_priors_path(&coreroom_dir, role));
    let priors = std::fs::read_to_string(&priors_path).map_err(|e| {
        anyhow::anyhow!(
            "could not read priors for @{role} at {}: {e} \
             (run `cr role list` to see existing roles)",
            priors_path.display()
        )
    })?;

    let resolved = resolve_all(&priors, project_root);
    if resolved.is_empty() {
        println!(
            "@{role} has no pointers in its priors file. \
             Add one with `[[<path>#L<n>-<m>@<sha>]]` or `[[<path>@HEAD]]`.\n\
             See `cr pointers --help` for the full grammar."
        );
        return Ok(());
    }
    println!("pointers in @{role} priors:");
    for r in &resolved {
        // Short locked SHA matches the short HEAD form used elsewhere,
        // so the line doesn't wrap on 80-col terminals and the two
        // SHAs are visually comparable.
        let display_pointer = Pointer {
            path: r.pointer.path.clone(),
            line_range: r.pointer.line_range,
            locked_sha: r
                .pointer
                .locked_sha
                .as_ref()
                .map(|s| s.chars().take(8).collect::<String>()),
        };
        let status_extra = match &r.status {
            PointerStatus::Fresh => String::new(),
            PointerStatus::Stale { head_sha } => format!(" (HEAD at {head_sha})"),
            PointerStatus::Unresolvable(reason) => match reason {
                UnresolvableReason::ShaNotFound { .. } => " (sha gone)".to_owned(),
                UnresolvableReason::NotAGitRepo { .. } => " (not a git repo)".to_owned(),
                UnresolvableReason::PathEscapesRepo { .. } => {
                    " (path escapes repo — security gate)".to_owned()
                }
                UnresolvableReason::PathNotFoundAtSha { .. } => " (path missing at sha)".to_owned(),
                _ => String::new(),
            },
        };
        println!(
            "  {} [[{display_pointer}]]  [{}{status_extra}]",
            status_glyph(&r.status),
            status_word(&r.status),
        );
        // For unresolvable pointers, print the actionable reason on a
        // second indented line so the user sees the remediation hint
        // without having to dig.
        if let PointerStatus::Unresolvable(reason) = &r.status {
            println!("      → {reason}");
        }
    }
    Ok(())
}

fn run_start(
    project: Option<PathBuf>,
    yolo: bool,
    fresh: bool,
    allow_large_priors: bool,
) -> Result<()> {
    if yolo && !confirm_yolo()? {
        return Ok(());
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let project_root = project_root_or_cwd(project)?;
        let options = coreroom::repl::RunOptions {
            permission_mode_override: yolo.then_some(PermissionMode::Bypass),
            fresh,
            allow_large_priors,
        };
        coreroom::repl::run_with_options(&project_root, options).await
    })
}

fn run_console_first_default() -> Result<()> {
    let project_root = project_root_or_cwd(None)?;
    if std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && project_root
            .join(coreroom::config::COREROOM_DIR)
            .join(coreroom::config::CONFIG_FILE)
            .is_file()
    {
        match coreroom::console_tui::run_live_console(&project_root) {
            Ok(()) => {
                eprintln!(
                    "CoreRoom console closed; starting REPL. Use `cr start` to skip the console."
                );
            }
            Err(error) => {
                eprintln!("CoreRoom console unavailable ({error:#}); starting REPL.");
            }
        }
    }
    run_start(Some(project_root), false, false, false)
}

fn run_console(project: Option<PathBuf>, snapshot: Option<PathBuf>) -> Result<()> {
    if let Some(snapshot) = snapshot {
        return coreroom::console_tui::run_snapshot_console(&snapshot);
    }
    let root = project_root_or_cwd(project)?;
    coreroom::console_tui::run_live_console(&root)
}

fn run_lock(project: Option<PathBuf>) -> Result<()> {
    let root = project_root_or_cwd(project)?;
    let coreroom_dir = root.join(coreroom::config::COREROOM_DIR);
    let lock = coreroom::lock::write(&coreroom_dir, coreroom::priors::ComposeOptions::default())?;
    println!(
        "✓ wrote {} ({} role{})",
        coreroom::lock::lock_path(&coreroom_dir).display(),
        lock.roles.len(),
        if lock.roles.len() == 1 { "" } else { "s" }
    );
    Ok(())
}

fn run_verify(project: Option<PathBuf>) -> Result<()> {
    let root = project_root_or_cwd(project)?;
    let coreroom_dir = root.join(coreroom::config::COREROOM_DIR);
    let report =
        coreroom::lock::verify(&coreroom_dir, coreroom::priors::ComposeOptions::default())?;
    if report.is_clean() {
        println!("✓ priors lock verified ({})", report.lock_path.display());
        return Ok(());
    }
    println!("priors lock drift detected:");
    for drift in &report.drifts {
        println!("  - {drift}");
    }
    bail!("run `cr lock` after reviewing intentional priors changes");
}

fn confirm_yolo() -> Result<bool> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Ok(true);
    }
    print!("Run this CoreRoom session with permission_mode=bypass for every role? [y/N] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes" | "YES"))
}

fn run_config_cmd(cmd: ConfigCmd) -> Result<()> {
    match cmd {
        ConfigCmd::Show { project } => coreroom::config_cmd::show(&project_root_or_cwd(project)?),
        ConfigCmd::Edit {
            project,
            user,
            local,
        } => {
            let layer = layer_from_flags(user, local);
            coreroom::config_cmd::edit(layer, &project_root_or_cwd(project)?)
        }
        ConfigCmd::Path {
            project,
            user,
            local,
        } => {
            let layer = layer_from_flags(user, local);
            coreroom::config_cmd::path(layer, &project_root_or_cwd(project)?)
        }
        ConfigCmd::Get { project, key } => {
            coreroom::config_cmd::get(&project_root_or_cwd(project)?, &key)
        }
        ConfigCmd::Set {
            project,
            project_layer,
            local,
            key,
            value,
        } => {
            let layer = if project_layer {
                LayerTarget::Project
            } else if local {
                LayerTarget::Local
            } else {
                LayerTarget::User
            };
            coreroom::config_cmd::set(layer, &project_root_or_cwd(project)?, &key, &value)
        }
    }
}

fn run_role_cmd(cmd: RoleCmd) -> Result<()> {
    match cmd {
        RoleCmd::Add {
            name,
            engine,
            model,
            project,
        } => {
            let root = project_root_or_cwd(project)?;
            coreroom::role::add(&root, &name, engine, model.as_deref())
        }
        RoleCmd::List { project } => coreroom::role::list(&project_root_or_cwd(project)?),
        RoleCmd::Show { name, project } => {
            coreroom::role::show(&project_root_or_cwd(project)?, &name)
        }
        RoleCmd::Attach {
            role,
            file_path,
            alias,
            project,
        } => coreroom::role::attach(
            &project_root_or_cwd(project)?,
            &role,
            &file_path,
            alias.as_deref(),
        ),
        RoleCmd::Detach {
            role,
            name,
            project,
        } => coreroom::role::detach(&project_root_or_cwd(project)?, &role, &name),
        RoleCmd::Knowledge {
            role,
            project,
            with_liveness,
        } => coreroom::role::knowledge(&project_root_or_cwd(project)?, &role, with_liveness),
        RoleCmd::Rm { name, project } => coreroom::role::rm(&project_root_or_cwd(project)?, &name),
        RoleCmd::Host { name, project } => {
            coreroom::role::set_host(&project_root_or_cwd(project)?, &name)
        }
        RoleCmd::SetOwner {
            name,
            owner,
            project,
        } => coreroom::role::set_owner(&project_root_or_cwd(project)?, &name, &owner),
        RoleCmd::SetAuthority {
            name,
            scopes,
            project,
        } => coreroom::role::set_authority(&project_root_or_cwd(project)?, &name, &scopes),
    }
}
