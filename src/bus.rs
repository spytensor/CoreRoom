//! Append-only message bus.
//!
//! [`MessageBus`] is the single source of truth for events emitted by every
//! role in a session. Durable [`CrepEvent`]s are:
//!
//! 1. Serialized to one line of JSON.
//! 2. Appended to the on-disk log at `.coderoom/messages.jsonl`.
//! 3. Broadcast to all live subscribers (the REPL renderer, future patch
//!    detectors, transcript writers, etc.).
//!
//! Live-only events, such as assistant text deltas, are broadcast without
//! being appended; the final `RoleSpoke` remains the durable transcript.
//!
//! Late subscribers do not see historical events; that's the job of
//! [`MessageBus::replay`] (and ultimately `cr show`), which streams the
//! existing on-disk log line-by-line.

use std::path::Path;

use fs2::FileExt;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{broadcast, Mutex};

use crate::crep::CrepEvent;

/// Capacity of the broadcast ring buffer. Late subscribers that fall this
/// far behind get a `RecvError::Lagged` and skip ahead — they have not
/// missed anything important since the on-disk log is the durable record.
const SUBSCRIBER_CAPACITY: usize = 1024;

/// Append-only event bus.
///
/// Construct one per `cr start` session. Multiple consumers can call
/// [`subscribe`](Self::subscribe) to observe events live; the durable log
/// at the configured path is always the source of truth.
pub struct MessageBus {
    file: Mutex<File>,
    tx: broadcast::Sender<CrepEvent>,
    live_tx: broadcast::Sender<CrepEvent>,
}

/// Result of replaying an on-disk JSONL log.
#[derive(Debug, Clone, PartialEq)]
pub struct Replay {
    /// Parsed CREP events, in file order.
    pub events: Vec<CrepEvent>,
    /// Number of non-empty lines that failed to parse as CREP.
    pub skipped_malformed: usize,
    /// 1-based line number of the first malformed line, if any.
    pub first_malformed_line: Option<usize>,
}

impl Replay {
    /// Whether replay yielded no valid events.
    ///
    /// Kept as a small compatibility affordance for call sites that used
    /// the old `Vec<CrepEvent>` return type directly.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Iterate over parsed events.
    pub fn iter(&self) -> std::slice::Iter<'_, CrepEvent> {
        self.events.iter()
    }
}

impl IntoIterator for Replay {
    type Item = CrepEvent;
    type IntoIter = std::vec::IntoIter<CrepEvent>;

    fn into_iter(self) -> Self::IntoIter {
        self.events.into_iter()
    }
}

impl<'a> IntoIterator for &'a Replay {
    type Item = &'a CrepEvent;
    type IntoIter = std::slice::Iter<'a, CrepEvent>;

    fn into_iter(self) -> Self::IntoIter {
        self.events.iter()
    }
}

impl std::fmt::Debug for MessageBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessageBus")
            .field("subscribers", &self.tx.receiver_count())
            .finish_non_exhaustive()
    }
}

impl MessageBus {
    /// Open (or create) the log at `path` and return a fresh bus.
    ///
    /// Existing log content is preserved; new events append after it.
    #[allow(clippy::unused_async)]
    pub async fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let std_file = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path.as_ref())?;
        std_file.lock_exclusive().map_err(|e| {
            std::io::Error::new(
                e.kind(),
                "another `cr` process is already attached to .coderoom/messages.jsonl in this \
                 project; close the other session before starting a new one",
            )
        })?;
        let file = File::from_std(std_file);
        let (tx, _initial) = broadcast::channel(SUBSCRIBER_CAPACITY);
        let (live_tx, _live_initial) = broadcast::channel(SUBSCRIBER_CAPACITY);
        Ok(Self {
            file: Mutex::new(file),
            tx,
            live_tx,
        })
    }

    /// Append the event to the log AND notify subscribers.
    ///
    /// Disk write happens first for durable events; if it fails the event
    /// is dropped and the error is returned. Live-only events skip disk
    /// and only hit subscribers.
    pub async fn publish(&self, event: CrepEvent) -> std::io::Result<()> {
        if is_durable_event(&event) {
            let serialized = serde_json::to_string(&event).map_err(std::io::Error::other)?;
            let mut line = serialized.into_bytes();
            line.push(b'\n');
            {
                let mut file = self.file.lock().await;
                file.write_all(&line).await?;
                file.flush().await?;
            }
        }
        // Sending to a broadcast channel with no live receivers returns
        // `Err(SendError)`; that's expected and not a publish failure.
        if is_durable_event(&event) {
            let _ = self.tx.send(event);
        } else {
            let _ = self.live_tx.send(event);
        }
        Ok(())
    }

    /// Subscribe to live events. Late subscribers see only events that
    /// arrive after this call.
    pub fn subscribe(&self) -> broadcast::Receiver<CrepEvent> {
        self.tx.subscribe()
    }

    /// Subscribe to live-only events such as assistant text deltas. These
    /// events are intentionally separated from the durable stream so a
    /// large token stream cannot evict final turn boundaries.
    pub fn subscribe_live(&self) -> broadcast::Receiver<CrepEvent> {
        self.live_tx.subscribe()
    }

    /// Stream every event currently on disk at `path`, in order, decoding
    /// each line as a [`CrepEvent`]. Malformed lines are counted and the
    /// first line number is returned to callers so `cr show` / `cr cost`
    /// can make corruption visible instead of silently producing an
    /// incomplete view.
    pub async fn replay(path: impl AsRef<Path>) -> std::io::Result<Replay> {
        let file = File::open(path.as_ref()).await?;
        let mut lines = BufReader::new(file).lines();
        let mut out = Vec::new();
        let mut skipped_malformed = 0usize;
        let mut first_malformed_line = None;
        let mut line_no = 0usize;
        while let Some(line) = lines.next_line().await? {
            line_no += 1;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<CrepEvent>(&line) {
                Ok(event) => out.push(event),
                Err(error) => {
                    skipped_malformed += 1;
                    first_malformed_line.get_or_insert(line_no);
                    tracing::warn!(%error, line = line_no, "skipping malformed JSONL line on replay");
                }
            }
        }
        Ok(Replay {
            events: out,
            skipped_malformed,
            first_malformed_line,
        })
    }

    /// Number of currently-active subscribers. Useful for diagnostics.
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

fn is_durable_event(event: &CrepEvent) -> bool {
    !matches!(event, CrepEvent::RoleOutputDelta { .. })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crep::StopReason;
    use pretty_assertions::assert_eq;

    fn sample_event(role: &str) -> CrepEvent {
        CrepEvent::RoleStarted {
            role: role.to_owned(),
            engine: "cc".to_owned(),
            model: "claude-opus-4-7".to_owned(),
            session_id: format!("session-{role}"),
            priors_hash: "dh1:0000".to_owned(),
        }
    }

    #[tokio::test]
    async fn publish_appends_line_to_log() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("messages.jsonl");
        let bus = MessageBus::open(&log).await.unwrap();

        bus.publish(sample_event("backend")).await.unwrap();
        bus.publish(sample_event("frontend")).await.unwrap();

        let content = tokio::fs::read_to_string(&log).await.unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        for line in &lines {
            let _: CrepEvent =
                serde_json::from_str(line).expect("each line round-trips as CrepEvent");
        }
    }

    #[tokio::test]
    async fn subscribers_receive_published_events() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("messages.jsonl");
        let bus = MessageBus::open(&log).await.unwrap();

        let mut rx_a = bus.subscribe();
        let mut rx_b = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 2);

        let event = sample_event("backend");
        bus.publish(event.clone()).await.unwrap();

        let recv_a = rx_a.recv().await.expect("subscriber A receives");
        let recv_b = rx_b.recv().await.expect("subscriber B receives");
        assert_eq!(recv_a, event);
        assert_eq!(recv_b, event);
    }

    #[tokio::test]
    async fn role_output_delta_is_broadcast_only() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("messages.jsonl");
        let bus = MessageBus::open(&log).await.unwrap();
        let mut durable_rx = bus.subscribe();
        let mut live_rx = bus.subscribe_live();

        let event = CrepEvent::RoleOutputDelta {
            role: "backend".into(),
            priors_hash: String::new(),
            text_delta: "partial".into(),
            sequence: 1,
            turn_id: String::new(),
            thread_id: String::new(),
        };
        bus.publish(event.clone()).await.unwrap();

        assert_eq!(live_rx.recv().await.unwrap(), event);
        assert!(durable_rx.try_recv().is_err());
        let content = tokio::fs::read_to_string(&log).await.unwrap_or_default();
        assert!(content.is_empty(), "delta should not be durable: {content}");
    }

    #[tokio::test]
    async fn open_preserves_existing_content() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("messages.jsonl");

        // First session writes one event then drops the bus.
        {
            let bus = MessageBus::open(&log).await.unwrap();
            bus.publish(sample_event("first")).await.unwrap();
        }

        // Second session opens the same log, writes another event.
        {
            let bus = MessageBus::open(&log).await.unwrap();
            bus.publish(sample_event("second")).await.unwrap();
        }

        let replayed = MessageBus::replay(&log).await.unwrap();
        assert_eq!(replayed.skipped_malformed, 0);
        assert_eq!(replayed.events.len(), 2);
        match (&replayed.events[0], &replayed.events[1]) {
            (CrepEvent::RoleStarted { role: r0, .. }, CrepEvent::RoleStarted { role: r1, .. }) => {
                assert_eq!(r0, "first");
                assert_eq!(r1, "second");
            }
            other => panic!("unexpected events: {other:?}"),
        }
    }

    #[tokio::test]
    async fn replay_skips_malformed_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("messages.jsonl");

        // Write a mix of valid and broken lines.
        let valid = serde_json::to_string(&sample_event("ok")).unwrap();
        let stopped = serde_json::to_string(&CrepEvent::RoleStopped {
            role: "ok".to_owned(),
            priors_hash: String::new(),
            reason: StopReason::Completed,
            turn_id: None,
        })
        .unwrap();
        let mixed = format!("{valid}\nthis-is-not-json\n\n{stopped}\n");
        tokio::fs::write(&log, mixed).await.unwrap();

        let replayed = MessageBus::replay(&log).await.unwrap();
        assert_eq!(replayed.events.len(), 2);
        assert_eq!(replayed.skipped_malformed, 1);
        assert_eq!(replayed.first_malformed_line, Some(2));
    }

    #[tokio::test]
    async fn debug_format_does_not_leak_file_internals() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("messages.jsonl");
        let bus = MessageBus::open(&log).await.unwrap();
        let dbg = format!("{bus:?}");
        assert!(dbg.contains("MessageBus"));
        assert!(
            !dbg.contains("File"),
            "Debug should not expose tokio::fs::File internals"
        );
    }

    #[tokio::test]
    async fn replay_handles_v0_1_and_v0_2_events_interleaved() {
        // Real upgrade story: a session log that started under v0.1
        // (no turn_id / thread_id fields) and continues under v0.2
        // (with the new fields plus a TurnDispatched / TurnInterrupted
        // pair) must replay end-to-end without losing events.
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("messages.jsonl");
        let body = r#"{"type":"role_started","role":"security","engine":"codex","model":"codex","session_id":"s1","priors_hash":"h"}
{"type":"role_spoke","role":"security","text":"legacy reply","mentions":[],"cost_usd":0.0,"cache_read":0}
{"type":"turn_dispatched","role":"security","turn_id":"tu-9","thread_id":"th-9","parent_turn_id":null,"queue_position":0}
{"type":"role_spoke","role":"security","text":"v0.2 reply","mentions":[],"cost_usd":0.0,"cache_read":0,"turn_id":"tu-9","thread_id":"th-9"}
{"type":"turn_interrupted","role":"security","turn_id":"tu-9","thread_id":"th-9","source":"user_halt","partial_text":null,"partial_mentions":[]}
{"type":"role_stopped","role":"security","reason":"refreshed"}
"#;
        tokio::fs::write(&log, body).await.unwrap();

        let replayed = MessageBus::replay(&log).await.unwrap();
        assert_eq!(replayed.skipped_malformed, 0);
        assert_eq!(replayed.events.len(), 6);

        // Legacy role_spoke deserializes with empty turn_id; v0.2 one
        // carries the real ids.
        match &replayed.events[1] {
            CrepEvent::RoleSpoke {
                turn_id, thread_id, ..
            } => {
                assert!(turn_id.is_empty());
                assert!(thread_id.is_empty());
            }
            other => panic!("expected legacy RoleSpoke, got {other:?}"),
        }
        match &replayed.events[3] {
            CrepEvent::RoleSpoke {
                turn_id, thread_id, ..
            } => {
                assert_eq!(turn_id, "tu-9");
                assert_eq!(thread_id, "th-9");
            }
            other => panic!("expected v0.2 RoleSpoke, got {other:?}"),
        }
        // v0.2 TurnDispatched and TurnInterrupted parse cleanly.
        assert!(matches!(
            &replayed.events[2],
            CrepEvent::TurnDispatched { turn_id, .. } if turn_id == "tu-9"
        ));
        assert!(matches!(
            &replayed.events[4],
            CrepEvent::TurnInterrupted { turn_id, source, .. }
                if turn_id == "tu-9" && *source == crate::crep::InterruptSource::UserHalt
        ));
    }
}
