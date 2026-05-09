//! Gemini engine adapter.
//!
//! v0.1 implementation is intentionally the simplest of the three: per
//! user message we spawn `gemini -p` once, capture its stdout, and emit
//! a single [`CrepEvent::RoleSpoke`]. There is no long-lived subprocess.
//!
//! Why so simple at v0.1:
//!
//! - Gemini's CLI does not expose a `--system-prompt` flag (only the
//!   `gemini skills` / `gemini extensions` subsystems load instructions
//!   automatically, and those are user-global rather than per-session).
//!   For v0.1 we prepend the composed priors to each user prompt with a
//!   fence separator. Inelegant, but functionally correct, and keeps the
//!   adapter trivially debuggable.
//! - `-y` (yolo) skips approval prompts, mirroring CC's
//!   `--dangerously-skip-permissions`. The wrapper-side gate over Gemini
//!   tool calls lands once we plumb its hook system (Gemini ships
//!   `gemini hooks migrate` that imports CC hook configs — same payload
//!   format).
//! - No streaming, no tool-call lifecycle, no multi-turn cache reuse.
//!   All three arrive in a follow-up PR (`feat/adapter-gemini-v2`)
//!   either via `gemini -o stream-json` (CC-shape) or `--experimental-acp`.
//!
//! `priors_hash` reuses [`crate::adapter::cc::fingerprint`] so the value
//! is comparable across engines.

use std::path::PathBuf;
use std::process::Stdio;

use tokio::process::Command;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::adapter::{
    AdapterError, AdapterResult, Engine, EngineAdapter, RoleConfig, RoleHandle, UserMessage,
};
use crate::crep::{CrepEvent, StopReason};

const CHANNEL_CAPACITY: usize = 32;

/// Adapter that drives the Gemini CLI in one-shot per-turn mode.
#[derive(Debug, Clone)]
pub struct GeminiAdapter {
    gemini_path: PathBuf,
}

impl Default for GeminiAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl GeminiAdapter {
    /// Construct an adapter that resolves `gemini` via the user's `PATH`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            gemini_path: PathBuf::from("gemini"),
        }
    }

    /// Construct an adapter pointing at a specific `gemini` binary.
    #[must_use]
    pub fn with_path(gemini_path: PathBuf) -> Self {
        Self { gemini_path }
    }
}

impl EngineAdapter for GeminiAdapter {
    fn engine(&self) -> Engine {
        Engine::Gemini
    }

    async fn start(&self, config: RoleConfig) -> AdapterResult<RoleHandle> {
        let priors_text = tokio::fs::read_to_string(&config.priors_path)
            .await
            .map_err(|source| AdapterError::PriorsRead {
                path: config.priors_path.clone(),
                source,
            })?;
        let priors_hash = crate::adapter::cc::fingerprint(&priors_text);

        let (tx_user, rx_user) = mpsc::channel::<UserMessage>(CHANNEL_CAPACITY);
        let (tx_events, rx_events) = mpsc::channel::<CrepEvent>(CHANNEL_CAPACITY);

        // Synthetic session id — Gemini's CLI offers --resume by index
        // for sessions persisted to disk, but we treat each turn as
        // independent at v0.1.
        let session_id = format!("gemini-{}", config.name);
        let _ = tx_events
            .send(CrepEvent::RoleStarted {
                role: config.name.clone(),
                engine: Engine::Gemini.as_str().to_owned(),
                model: config
                    .model
                    .clone()
                    .unwrap_or_else(|| "gemini".to_owned()),
                session_id,
                priors_hash,
            })
            .await;

        tokio::spawn(per_turn_loop(
            self.gemini_path.clone(),
            config.name.clone(),
            config.model.clone(),
            priors_text,
            rx_user,
            tx_events,
        ));

        Ok(RoleHandle {
            role: config.name,
            engine: Engine::Gemini,
            tx_user,
            rx_events,
        })
    }
}

async fn per_turn_loop(
    gemini_path: PathBuf,
    role: String,
    model: Option<String>,
    priors_text: String,
    mut rx: mpsc::Receiver<UserMessage>,
    events: mpsc::Sender<CrepEvent>,
) {
    while let Some(msg) = rx.recv().await {
        let UserMessage::Prompt(prompt) = msg else {
            continue;
        };
        match run_one_turn(&gemini_path, model.as_deref(), &priors_text, &prompt).await {
            Ok(text) => {
                let mentions = crate::adapter::cc::parse_mentions(&text);
                let _ = events
                    .send(CrepEvent::RoleSpoke {
                        role: role.clone(),
                        text,
                        mentions,
                        cost_usd: 0.0,
                        cache_read: 0,
                    })
                    .await;
            }
            Err(error) => {
                warn!(role, %error, "gemini turn failed");
                let _ = events
                    .send(CrepEvent::RoleSpoke {
                        role: role.clone(),
                        text: format!("[gemini error: {error}]"),
                        mentions: Vec::new(),
                        cost_usd: 0.0,
                        cache_read: 0,
                    })
                    .await;
            }
        }
    }
    let _ = events
        .send(CrepEvent::RoleStopped {
            role,
            reason: StopReason::Completed,
        })
        .await;
    debug!("gemini per-turn loop exiting");
}

/// Run a single `gemini -p` invocation with priors prepended and return
/// its stdout (text mode).
async fn run_one_turn(
    gemini_path: &PathBuf,
    model: Option<&str>,
    priors_text: &str,
    user_prompt: &str,
) -> std::io::Result<String> {
    let combined = format!("{priors_text}\n\n---\n\n{user_prompt}");

    let mut cmd = Command::new(gemini_path);
    cmd.arg("-p")
        .arg(&combined)
        .arg("--output-format")
        .arg("text")
        .arg("-y")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(model) = model {
        cmd.arg("--model").arg(model);
    }

    let output = cmd.output().await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(std::io::Error::other(format!(
            "gemini exited with {}: {stderr}",
            output.status
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn engine_id() {
        let adapter = GeminiAdapter::new();
        assert_eq!(adapter.engine(), Engine::Gemini);
    }

    #[test]
    fn with_path_overrides_default() {
        let adapter = GeminiAdapter::with_path(PathBuf::from("/opt/gemini"));
        assert_eq!(adapter.gemini_path, PathBuf::from("/opt/gemini"));
    }
}
