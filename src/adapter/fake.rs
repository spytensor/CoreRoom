//! Deterministic local engine adapter for dogfood and tests.
//!
//! The fake engine intentionally sits behind an explicit environment
//! gate. It is an `EngineAdapter` so the runtime, bus, TUI, permission
//! bridge, and transcript paths treat it like a normal role engine, but
//! production configs cannot opt into it accidentally.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::json;
use tokio::sync::{mpsc, oneshot};

use crate::adapter::{
    AdapterError, AdapterResult, CompactResult, Engine, EngineAdapter, PromptMessage, RoleConfig,
    RoleHandle, UserMessage,
};
use crate::crep::{CrepEvent, InterruptSource, StopReason, TurnOutcome};
use crate::permissions::{DecisionScope, PermissionDecision as BridgeDecision};
use crate::turn::TurnId;

/// Enables `engine = "fake"` in project config.
pub const ENABLE_ENV: &str = "COREROOM_ENABLE_FAKE_ENGINE";
/// Optional deterministic response text.
pub const RESPONSE_ENV: &str = "COREROOM_FAKE_ENGINE_RESPONSE";
/// Optional chunk delay in milliseconds.
pub const CHUNK_MS_ENV: &str = "COREROOM_FAKE_ENGINE_CHUNK_MS";

const CHANNEL_CAPACITY: usize = 32;
const DEFAULT_RESPONSE: &str = "fake-stream-1 fake-stream-2 fake-stream-3";
const DEFAULT_CHUNK_MS: u64 = 80;

/// Whether the fake engine is explicitly enabled for this process.
#[must_use]
pub fn enabled() -> bool {
    std::env::var(ENABLE_ENV).is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

/// Adapter that emits deterministic CREP without spawning a networked CLI.
#[derive(Debug, Default, Clone, Copy)]
pub struct FakeAdapter;

impl FakeAdapter {
    /// Construct a fake adapter.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl EngineAdapter for FakeAdapter {
    fn engine(&self) -> Engine {
        Engine::Fake
    }

    async fn start(&self, config: RoleConfig) -> AdapterResult<RoleHandle> {
        if !enabled() {
            return Err(AdapterError::Engine {
                engine: Engine::Fake.as_str(),
                message: format!(
                    "engine=\"fake\" is dogfood/test-only; set {ENABLE_ENV}=1 to enable it"
                ),
            });
        }

        let priors_text = tokio::fs::read_to_string(&config.priors_path)
            .await
            .map_err(|source| AdapterError::PriorsRead {
                path: config.priors_path.clone(),
                source,
            })?;
        let priors_hash = crate::adapter::cc::fingerprint(&priors_text);
        let (tx_user, rx_user) = mpsc::channel::<UserMessage>(CHANNEL_CAPACITY);
        let (tx_events, rx_events) = mpsc::channel::<CrepEvent>(CHANNEL_CAPACITY);
        let (stop_tx, stop_rx) = oneshot::channel::<StopReason>();
        let (interrupt_tx, interrupt_rx) =
            mpsc::channel::<TurnId>(crate::adapter::INTERRUPT_CHANNEL_CAPACITY);

        let session_id = config
            .resume_session_id
            .clone()
            .unwrap_or_else(|| format!("fake-{}", config.name));
        let _ = tx_events
            .send(CrepEvent::RoleStarted {
                role: config.name.clone(),
                engine: Engine::Fake.as_str().to_owned(),
                model: config.model.clone().unwrap_or_else(|| "fake".to_owned()),
                session_id,
                priors_hash,
            })
            .await;

        tokio::spawn(
            FakeLoop {
                role: config.name.clone(),
                permission_socket_path: config.permission_socket_path.clone(),
                rx: rx_user,
                events: tx_events,
                stop_rx,
                interrupt_rx,
            }
            .run(),
        );

        Ok(RoleHandle::new(
            config.name,
            Engine::Fake,
            tx_user,
            rx_events,
            stop_tx,
            interrupt_tx,
        ))
    }
}

#[derive(Debug)]
struct FakeLoop {
    role: String,
    permission_socket_path: Option<PathBuf>,
    rx: mpsc::Receiver<UserMessage>,
    events: mpsc::Sender<CrepEvent>,
    stop_rx: oneshot::Receiver<StopReason>,
    interrupt_rx: mpsc::Receiver<TurnId>,
}

impl FakeLoop {
    async fn run(mut self) {
        loop {
            tokio::select! {
                reason = &mut self.stop_rx => {
                    self.emit_stopped(reason.unwrap_or(StopReason::Completed), None).await;
                    break;
                }
                message = self.rx.recv() => {
                    let Some(message) = message else {
                        self.emit_stopped(StopReason::Completed, None).await;
                        break;
                    };
                    match message {
                        UserMessage::Prompt(prompt) => self.handle_prompt(prompt).await,
                        UserMessage::CompactContext { respond_to } => {
                            let _ = respond_to.send(CompactResult::Unsupported {
                                reason: "fake engine has no native compact context".to_owned(),
                            });
                        }
                        UserMessage::ToolDecision { .. } => {}
                    }
                }
            }
        }
    }

    async fn handle_prompt(&mut self, prompt: PromptMessage) {
        let _ = self
            .events
            .send(CrepEvent::WorkTitle {
                role: self.role.clone(),
                priors_hash: String::new(),
                title: "Fake engine dogfood turn".to_owned(),
                turn_id: prompt.turn_id.clone(),
                thread_id: prompt.thread_id.clone(),
            })
            .await;

        let permission_result = if prompt.text.to_ascii_lowercase().contains("fake permission") {
            Some(self.run_permission_probe(&prompt).await)
        } else {
            None
        };
        let response = response_text(permission_result.as_ref());
        let mut partial = String::new();
        let mut interrupt_rx_closed = false;
        for (index, chunk) in response_chunks(&response).into_iter().enumerate() {
            let sleep = tokio::time::sleep(chunk_delay());
            tokio::pin!(sleep);
            loop {
                tokio::select! {
                    () = &mut sleep => {
                        break;
                    }
                    interrupt = self.interrupt_rx.recv(), if !interrupt_rx_closed => {
                        match interrupt {
                            Some(interrupt) if interrupt == prompt.turn_id => {
                                let _ = self.events.send(CrepEvent::TurnInterrupted {
                                    role: self.role.clone(),
                                    priors_hash: String::new(),
                                    turn_id: prompt.turn_id,
                                    thread_id: prompt.thread_id,
                                    source: InterruptSource::UserHalt,
                                    partial_text: (!partial.is_empty()).then_some(partial),
                                    partial_mentions: Vec::new(),
                                }).await;
                                return;
                            }
                            Some(_) => {}
                            None => {
                                interrupt_rx_closed = true;
                            }
                        }
                    }
                }
            }
            partial.push_str(&chunk);
            let _ = self
                .events
                .send(CrepEvent::RoleOutputDelta {
                    role: self.role.clone(),
                    priors_hash: String::new(),
                    text_delta: chunk,
                    sequence: u64::try_from(index).unwrap_or(u64::MAX),
                    turn_id: prompt.turn_id.clone(),
                    thread_id: prompt.thread_id.clone(),
                })
                .await;
        }
        let _ = self
            .events
            .send(CrepEvent::RoleSpoke {
                role: self.role.clone(),
                priors_hash: String::new(),
                text: response,
                mentions: Vec::new(),
                cost_usd: 0.0,
                cache_read: 0,
                turn_id: prompt.turn_id,
                thread_id: prompt.thread_id,
                outcome: TurnOutcome::Continue,
                phase_block: None,
            })
            .await;
    }

    async fn run_permission_probe(&mut self, prompt: &PromptMessage) -> PermissionProbeResult {
        let tool_use_id = format!("fake-tool-{}", prompt.turn_id);
        let input = json!({"command": "fake permission dogfood"});
        let _ = self
            .events
            .send(CrepEvent::ToolCallProposed {
                role: self.role.clone(),
                priors_hash: String::new(),
                tool_name: "FakeTool".to_owned(),
                tool_input: input.clone(),
                tool_use_id: tool_use_id.clone(),
                turn_id: prompt.turn_id.clone(),
                thread_id: prompt.thread_id.clone(),
            })
            .await;

        let Some(socket_path) = self.permission_socket_path.clone() else {
            let reason = "fake permission probe had no live bridge".to_owned();
            self.emit_permission_denied(prompt, &tool_use_id, &input, &reason)
                .await;
            return PermissionProbeResult::Denied;
        };
        match crate::permissions::bridge::request_decision_async(
            &socket_path,
            &self.role,
            "FakeTool",
            &input,
            "fake engine dogfood permission probe",
        )
        .await
        {
            Ok(response) if response.decision == BridgeDecision::Allow => {
                let _ = self
                    .events
                    .send(CrepEvent::ToolCallExecuted {
                        role: self.role.clone(),
                        priors_hash: String::new(),
                        tool_use_id,
                        ok: true,
                        output_summary: match response.scope {
                            DecisionScope::Once => "fake permission allowed once",
                            DecisionScope::Session => "fake permission allowed for session",
                        }
                        .to_owned(),
                        turn_id: prompt.turn_id.clone(),
                        thread_id: prompt.thread_id.clone(),
                    })
                    .await;
                PermissionProbeResult::Allowed
            }
            Ok(response) => {
                self.emit_permission_denied(prompt, &tool_use_id, &input, &response.reason)
                    .await;
                PermissionProbeResult::Denied
            }
            Err(error) => {
                let reason = format!("fake permission bridge failed: {error}");
                self.emit_permission_denied(prompt, &tool_use_id, &input, &reason)
                    .await;
                PermissionProbeResult::Denied
            }
        }
    }

    async fn emit_permission_denied(
        &self,
        prompt: &PromptMessage,
        tool_use_id: &str,
        input: &serde_json::Value,
        reason: &str,
    ) {
        let _ = tool_use_id;
        let _ = self
            .events
            .send(CrepEvent::PermissionDenied {
                role: self.role.clone(),
                priors_hash: String::new(),
                tool_name: "FakeTool".to_owned(),
                tool_input: input.clone(),
                reason: reason.to_owned(),
                turn_id: prompt.turn_id.clone(),
                thread_id: prompt.thread_id.clone(),
            })
            .await;
    }

    async fn emit_stopped(&self, reason: StopReason, turn_id: Option<TurnId>) {
        let _ = self
            .events
            .send(CrepEvent::RoleStopped {
                role: self.role.clone(),
                priors_hash: String::new(),
                reason,
                turn_id,
            })
            .await;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PermissionProbeResult {
    Allowed,
    Denied,
}

fn response_text(permission_result: Option<&PermissionProbeResult>) -> String {
    match permission_result {
        Some(PermissionProbeResult::Allowed) => {
            "fake-permission-allowed fake-stream-1 fake-stream-2 fake-stream-3".to_owned()
        }
        Some(PermissionProbeResult::Denied) => {
            "fake-permission-denied fake-stream-1 fake-stream-2 fake-stream-3".to_owned()
        }
        None => std::env::var(RESPONSE_ENV).unwrap_or_else(|_| DEFAULT_RESPONSE.to_owned()),
    }
}

fn response_chunks(response: &str) -> Vec<String> {
    response
        .split_whitespace()
        .map(|word| format!("{word} "))
        .collect()
}

fn chunk_delay() -> Duration {
    let millis = std::env::var(CHUNK_MS_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_CHUNK_MS);
    Duration::from_millis(millis)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{PermissionMode, RoleConfig};
    use std::sync::OnceLock;
    use tempfile::NamedTempFile;
    use tokio::sync::Mutex;

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[tokio::test]
    async fn fake_adapter_requires_explicit_env_gate() {
        let _guard = env_lock().lock().await;
        std::env::remove_var(ENABLE_ENV);
        let priors = NamedTempFile::new().expect("priors");
        let err = FakeAdapter::new()
            .start(role_config(priors.path().to_path_buf()))
            .await
            .expect_err("fake engine should be gated");
        assert!(err.to_string().contains(ENABLE_ENV));
    }

    #[tokio::test]
    async fn fake_adapter_streams_chunks_and_final_turn() {
        let _guard = env_lock().lock().await;
        std::env::set_var(ENABLE_ENV, "1");
        std::env::set_var(RESPONSE_ENV, "alpha beta");
        std::env::set_var(CHUNK_MS_ENV, "1");
        let priors = NamedTempFile::new().expect("priors");
        let handle = FakeAdapter::new()
            .start(role_config(priors.path().to_path_buf()))
            .await
            .expect("fake starts");
        let crate::adapter::RoleHandleParts {
            tx_user,
            mut rx_events,
            stop_tx: _stop_tx,
            interrupt_tx: _interrupt_tx,
            ..
        } = handle.into_parts();
        tx_user
            .send(UserMessage::prompt("hello", "turn-test", "thread-test"))
            .await
            .expect("prompt send");

        let mut deltas = Vec::new();
        let mut final_text = None;
        while let Some(event) = rx_events.recv().await {
            match event {
                CrepEvent::RoleOutputDelta { text_delta, .. } => deltas.push(text_delta),
                CrepEvent::RoleSpoke { text, .. } => {
                    final_text = Some(text);
                    break;
                }
                _ => {}
            }
        }
        assert_eq!(deltas, vec!["alpha ".to_owned(), "beta ".to_owned()]);
        assert_eq!(final_text.as_deref(), Some("alpha beta"));
        std::env::remove_var(ENABLE_ENV);
        std::env::remove_var(RESPONSE_ENV);
        std::env::remove_var(CHUNK_MS_ENV);
    }

    fn role_config(priors_path: PathBuf) -> RoleConfig {
        RoleConfig {
            name: "host".to_owned(),
            engine: Engine::Fake,
            model: None,
            priors_path,
            permission_mode: PermissionMode::Bypass,
            permission_policy_path: None,
            permission_socket_path: None,
            resume_session_id: None,
        }
    }
}
