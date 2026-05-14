//! Two-phase write-ahead log for turn dispatch (amendment A-012).
//!
//! The REPL writes a [`CrepEvent::TurnIntent`] to the durable bus
//! immediately before handing a brief to a role's adapter, and a
//! matching [`CrepEvent::TurnCommit`] immediately after observing the
//! turn's terminal event (`RoleSpoke`, `TurnInterrupted`, or
//! `RoleStopped`). The pairing is keyed by `turn_id`; an intent with
//! no commit is an "orphan turn" surfaced through `cr show --orphans`
//! and `cr doctor`.
//!
//! `parent_hash` on `TurnIntent` references the `payload_sha` of the
//! most recent `TurnCommit` on the same `thread_id`, forming a
//! per-thread chain the orphan scan can verify.
//!
//! Hash format: digests use the existing
//! [`crate::adapter::cc::fingerprint`] helper (`dh1:` prefix on a
//! non-cryptographic `DefaultHasher`). The WAL only needs
//! collision-resistant pairing within a single REPL process;
//! reproducibility across Rust releases is A-008's concern, not this
//! module's.

use std::collections::HashMap;

use crate::crep::CrepEvent;
use crate::turn::TurnId;

/// Content digest used for `TurnIntent.intent_sha`.
#[must_use]
pub fn intent_sha(brief: &str) -> String {
    crate::adapter::cc::fingerprint(brief)
}

/// Content digest used for `TurnCommit.payload_sha`.
#[must_use]
pub fn payload_sha(payload: &str) -> String {
    crate::adapter::cc::fingerprint(payload)
}

/// Tracks the latest committed `payload_sha` per thread so successive
/// `TurnIntent` events on the same thread can chain via `parent_hash`.
#[derive(Debug, Clone, Default)]
pub struct ChainTracker {
    by_thread: HashMap<TurnId, String>,
}

impl ChainTracker {
    /// Fresh tracker with no observed commits.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that `payload_sha` committed on `thread_id`.
    pub fn observe_commit(&mut self, thread_id: &TurnId, payload_sha: String) {
        self.by_thread.insert(thread_id.clone(), payload_sha);
    }

    /// `payload_sha` of the most recent commit on `thread_id`, or
    /// `None` if no commit has landed on that thread yet.
    #[must_use]
    pub fn parent_hash_for(&self, thread_id: &TurnId) -> Option<String> {
        self.by_thread.get(thread_id).cloned()
    }

    /// Replay a slice of CREP events and absorb every `TurnCommit` so a
    /// tracker rehydrated from `messages.jsonl` has the same per-thread
    /// state the live REPL would have built up.
    pub fn ingest_replay<'a, I>(&mut self, events: I)
    where
        I: IntoIterator<Item = &'a CrepEvent>,
    {
        for event in events {
            if let CrepEvent::TurnCommit {
                thread_id,
                payload_sha,
                ..
            } = event
            {
                self.observe_commit(thread_id, payload_sha.clone());
            }
        }
    }
}

/// A WAL pair found during orphan analysis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrphanTurn {
    /// Role whose intent was never committed.
    pub role: String,
    /// `turn_id` of the orphan intent.
    pub turn_id: TurnId,
    /// `thread_id` the intent was filed on.
    pub thread_id: TurnId,
    /// `intent_sha` of the brief the REPL was about to send.
    pub intent_sha: String,
    /// `parent_hash` the intent referenced, if any.
    pub parent_hash: Option<String>,
}

/// Scan a slice of CREP events for `TurnIntent`s without a matching
/// `TurnCommit`. Returns orphans in the order their intents appear.
#[must_use]
pub fn scan_orphans(events: &[CrepEvent]) -> Vec<OrphanTurn> {
    let mut pending: Vec<OrphanTurn> = Vec::new();
    let mut committed: std::collections::HashSet<&TurnId> = std::collections::HashSet::new();

    for event in events {
        match event {
            CrepEvent::TurnIntent {
                role,
                turn_id,
                thread_id,
                parent_hash,
                intent_sha,
            } => {
                pending.push(OrphanTurn {
                    role: role.clone(),
                    turn_id: turn_id.clone(),
                    thread_id: thread_id.clone(),
                    intent_sha: intent_sha.clone(),
                    parent_hash: parent_hash.clone(),
                });
            }
            CrepEvent::TurnCommit { turn_id, .. } => {
                committed.insert(turn_id);
            }
            _ => {}
        }
    }

    pending
        .into_iter()
        .filter(|intent| !committed.contains(&intent.turn_id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn intent(role: &str, turn: &str, thread: &str, parent: Option<&str>, sha: &str) -> CrepEvent {
        CrepEvent::TurnIntent {
            role: role.to_owned(),
            turn_id: turn.to_owned(),
            thread_id: thread.to_owned(),
            parent_hash: parent.map(str::to_owned),
            intent_sha: sha.to_owned(),
        }
    }

    fn commit(role: &str, turn: &str, thread: &str, sha: &str) -> CrepEvent {
        CrepEvent::TurnCommit {
            role: role.to_owned(),
            turn_id: turn.to_owned(),
            thread_id: thread.to_owned(),
            payload_sha: sha.to_owned(),
            priors_hash: String::new(),
        }
    }

    #[test]
    fn intent_sha_is_deterministic_for_same_input() {
        assert_eq!(intent_sha("hello"), intent_sha("hello"));
        assert_ne!(intent_sha("hello"), intent_sha("hello!"));
    }

    #[test]
    fn intent_and_payload_use_same_format() {
        let a = intent_sha("brief");
        let b = payload_sha("brief");
        assert_eq!(a, b);
        assert!(a.starts_with("dh1:"), "format prefix changed: {a}");
    }

    #[test]
    fn chain_tracker_observes_and_returns_parent() {
        let mut tracker = ChainTracker::new();
        assert!(tracker.parent_hash_for(&"th-1".to_owned()).is_none());

        tracker.observe_commit(&"th-1".to_owned(), "dh1:aa".to_owned());
        assert_eq!(
            tracker.parent_hash_for(&"th-1".to_owned()),
            Some("dh1:aa".to_owned())
        );

        tracker.observe_commit(&"th-2".to_owned(), "dh1:bb".to_owned());
        assert_eq!(
            tracker.parent_hash_for(&"th-1".to_owned()),
            Some("dh1:aa".to_owned())
        );

        tracker.observe_commit(&"th-1".to_owned(), "dh1:cc".to_owned());
        assert_eq!(
            tracker.parent_hash_for(&"th-1".to_owned()),
            Some("dh1:cc".to_owned())
        );
    }

    #[test]
    fn chain_tracker_ingest_replay_rebuilds_state() {
        let events = vec![
            intent("backend", "tu-1", "th-1", None, "dh1:i1"),
            commit("backend", "tu-1", "th-1", "dh1:p1"),
            intent("backend", "tu-2", "th-1", Some("dh1:p1"), "dh1:i2"),
            commit("backend", "tu-2", "th-1", "dh1:p2"),
        ];

        let mut tracker = ChainTracker::new();
        tracker.ingest_replay(events.iter());
        assert_eq!(
            tracker.parent_hash_for(&"th-1".to_owned()),
            Some("dh1:p2".to_owned())
        );
    }

    #[test]
    fn scan_orphans_returns_unmatched_intents() {
        let events = vec![
            intent("backend", "tu-1", "th-1", None, "dh1:i1"),
            commit("backend", "tu-1", "th-1", "dh1:p1"),
            intent("backend", "tu-2", "th-1", Some("dh1:p1"), "dh1:i2"),
        ];

        let orphans = scan_orphans(&events);
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].role, "backend");
        assert_eq!(orphans[0].turn_id, "tu-2");
        assert_eq!(orphans[0].parent_hash.as_deref(), Some("dh1:p1"));
    }

    #[test]
    fn scan_orphans_empty_when_all_pairs_complete() {
        let events = vec![
            intent("backend", "tu-1", "th-1", None, "dh1:i1"),
            commit("backend", "tu-1", "th-1", "dh1:p1"),
            intent("frontend", "tu-2", "th-2", None, "dh1:i2"),
            commit("frontend", "tu-2", "th-2", "dh1:p2"),
        ];
        assert!(scan_orphans(&events).is_empty());
    }

    #[test]
    fn scan_orphans_preserves_intent_order() {
        let events = vec![
            intent("backend", "tu-1", "th-1", None, "dh1:i1"),
            intent("frontend", "tu-2", "th-2", None, "dh1:i2"),
        ];
        let orphans = scan_orphans(&events);
        assert_eq!(orphans.len(), 2);
        assert_eq!(orphans[0].turn_id, "tu-1");
        assert_eq!(orphans[1].turn_id, "tu-2");
    }

    #[test]
    fn scan_orphans_ignores_non_wal_events() {
        let events = vec![CrepEvent::RoleStarted {
            role: "backend".into(),
            engine: "cc".into(),
            model: "m".into(),
            session_id: "s".into(),
            priors_hash: "p".into(),
        }];
        assert!(scan_orphans(&events).is_empty());
    }
}
