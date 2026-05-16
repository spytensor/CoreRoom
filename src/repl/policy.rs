//! Auto-routing policy for `send_and_drain`.
//!
//! Amendment A-007 extracts the routing decision out of the dispatcher
//! into a single-method trait. The dispatcher pops a finished turn off
//! the worklist, asks the policy what to do with the turn's mentions,
//! and forwards the result. All conversation-level decisions —
//! grounding gate, `cr-status:` outcome short-circuit, self/unknown
//! filtering — live behind this trait.
//!
//! [`DefaultPolicy`] reproduces the exact routing behaviour
//! `send_and_drain` had before A-007 *plus* the new outcome-based
//! short-circuit. Future policies (e.g. "discussion class auto-routes
//! at most one peer") implement the same trait without touching the
//! dispatcher.

use crate::crep::TurnOutcome;

use super::turn::CapturedTurn;

/// What the dispatcher should do after a turn finishes.
///
/// Empty `targets` ends the chain. When `skip_note` is set *and* the
/// chain ends here, the dispatcher renders it as a single dim line so
/// the user understands why a reply with peer mentions did not produce
/// follow-up turns. Notes are otherwise silent — a normal "no mentions
/// to route" turn doesn't need to explain itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RoutingDecision {
    /// Roles to dispatch as follow-up turns, in order. Pushed onto the
    /// worklist verbatim; duplicates are preserved by design (see
    /// `send_and_drain` comment).
    pub(super) targets: Vec<String>,
    /// Optional one-line explanation when the policy elects not to fan
    /// out a reply that did contain `@`-mentions. Rendered with the
    /// same italic FADE-arrow style as today's grounding-gate hint.
    pub(super) skip_note: Option<String>,
}

impl RoutingDecision {
    pub(super) fn fan_out(targets: Vec<String>) -> Self {
        Self {
            targets,
            skip_note: None,
        }
    }

    pub(super) fn skip(note: impl Into<String>) -> Self {
        Self {
            targets: Vec::new(),
            skip_note: Some(note.into()),
        }
    }

    pub(super) fn silent_stop() -> Self {
        Self {
            targets: Vec::new(),
            skip_note: None,
        }
    }
}

/// Read-only view of the current routing chain. Default policy ignores
/// it; trait method takes it so future policies (e.g. "discussion-class
/// task: at most one consultation per thread") can inspect chain state
/// without further dispatcher surgery.
///
/// `thread_id`, `originator_role`, and the chain-aware `thread` field
/// on [`TurnContext`] are dead today by design — they are the seams a
/// future policy plugs into, and removing them would just force the
/// next amendment to re-thread the same data through `send_and_drain`.
#[derive(Debug, Clone, Default)]
#[allow(
    dead_code,
    reason = "fields are extension seams for future DispatchPolicy implementations (A-007)"
)]
pub(super) struct ThreadView {
    /// Stable thread id shared by every turn in the chain (see
    /// `crate::turn::new_thread_id`). Empty when the dispatcher has
    /// not yet wired ids end-to-end (today's adapters still emit
    /// `LEGACY_TURN_ID`).
    pub(super) thread_id: String,
    /// How many turns the chain has dispatched so far, *including* the
    /// finished one. Diagnostic, not a cap.
    pub(super) hops: usize,
    /// Role that received the user's initial dispatch. Useful for
    /// "synthesis back to originator" logic.
    pub(super) originator_role: String,
    /// Outcomes of recent turns in the chain (most recent last). Bounded
    /// length kept by the dispatcher; default policy doesn't read it.
    pub(super) recent_outcomes: Vec<TurnOutcome>,
}

/// Per-call context passed to [`DispatchPolicy::route_after_turn`].
/// Bundles the finished turn, the rest of the chain state, and the
/// list of currently-running roles so the policy can filter against
/// the live roster.
#[derive(Debug)]
pub(super) struct TurnContext<'a> {
    pub(super) current_role: &'a str,
    pub(super) captured: &'a CapturedTurn,
    pub(super) known_roles: &'a [&'a str],
    /// Read-only chain state. Dead in the default policy today; see
    /// [`ThreadView`] for the rationale on keeping the seam wired.
    #[allow(
        dead_code,
        reason = "extension seam for future DispatchPolicy implementations (A-007)"
    )]
    pub(super) thread: &'a ThreadView,
}

/// Decide what to do with a finished turn's `@`-mentions.
///
/// One method, one return — the entire routing decision surface for
/// `send_and_drain`. Implementations should be pure: take the context
/// in, return a decision, do not perform I/O. The dispatcher renders
/// any `skip_note` itself.
pub(super) trait DispatchPolicy {
    fn route_after_turn(&self, ctx: &TurnContext<'_>) -> RoutingDecision;
}

/// Default policy: reproduces the pre-A-007 routing behaviour and adds
/// the structured-outcome short-circuit on top.
///
/// Decision order matters:
///
/// 1. **Grounding gate.** If the role's tool calls were systematically
///    denied (and it still mentioned peers), drop the chain with a
///    diagnostic. An "ungrounded" outcome claim is itself suspect —
///    the role couldn't read the repo, so any `cr-status: converged`
///    it appended is also a guess. Gate runs first.
/// 2. **Outcome short-circuit.** Anything other than
///    [`TurnOutcome::Continue`] ends the chain. A skip note fires only
///    when the reply *did* contain mentions; a silent stop ("the role
///    just answered, period") needs no narration.
/// 3. **Filter and fan out.** Today's `filter_routable_mentions`:
///    drop self-mentions, drop unknown roles, preserve duplicates.
pub(super) struct DefaultPolicy;

impl DispatchPolicy for DefaultPolicy {
    fn route_after_turn(&self, ctx: &TurnContext<'_>) -> RoutingDecision {
        if ctx.captured.activity.looks_ungrounded() && !ctx.captured.mentions.is_empty() {
            return RoutingDecision::skip(ungrounded_skip_message(
                ctx.current_role,
                &ctx.captured.activity,
            ));
        }

        if let Some(label) = terminating_outcome_label(ctx.captured.outcome) {
            return if ctx.captured.mentions.is_empty() {
                RoutingDecision::silent_stop()
            } else {
                RoutingDecision::skip(format!(
                    "@{current} declared {label} — not routing this reply's mentions",
                    current = ctx.current_role,
                ))
            };
        }

        let targets = filter_routable_mentions(
            ctx.current_role,
            &ctx.captured.mentions,
            ctx.known_roles,
        );
        RoutingDecision::fan_out(targets)
    }
}

fn terminating_outcome_label(outcome: TurnOutcome) -> Option<&'static str> {
    match outcome {
        TurnOutcome::Continue => None,
        TurnOutcome::NoIncrement => Some("no_increment"),
        TurnOutcome::Converged => Some("converged"),
        TurnOutcome::NeedsUser => Some("needs_user"),
    }
}

/// Drop self-mentions and unknown-role mentions; preserve order and
/// duplicates. Same semantics A-005 locked in.
pub(super) fn filter_routable_mentions(
    current_role: &str,
    mentions: &[String],
    known_roles: &[&str],
) -> Vec<String> {
    mentions
        .iter()
        .filter(|m| m.as_str() != current_role)
        .filter(|m| known_roles.contains(&m.as_str()))
        .cloned()
        .collect()
}

/// Build the diagnostic line shown when the grounding gate elects to
/// drop a reply's mentions. Mirrors the pre-A-007 inline message so
/// the user-visible string is unchanged.
fn ungrounded_skip_message(
    current_role: &str,
    activity: &super::turn::TurnActivity,
) -> String {
    let suggestion = if activity.denied > 0 {
        let names = activity.top_denied_tools(3).join(", ");
        let primary = activity
            .top_denied_tools(1)
            .first()
            .cloned()
            .unwrap_or_else(|| names.clone());
        format!(" — try /allow {primary} or `cr start --yolo`")
    } else {
        String::new()
    };
    let summary = if activity.denied > 0 {
        format!("{} permission denial(s)", activity.denied)
    } else {
        format!("all {} tool calls failed", activity.proposed)
    };
    format!("skipping auto-route: @{current_role} had {summary} this turn{suggestion}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repl::turn::TurnActivity;
    use std::collections::BTreeMap;

    fn captured(text: &str, mentions: Vec<&str>, outcome: TurnOutcome) -> CapturedTurn {
        CapturedTurn {
            text: text.into(),
            mentions: mentions.into_iter().map(String::from).collect(),
            activity: TurnActivity::default(),
            outcome,
        }
    }

    fn ungrounded_activity() -> TurnActivity {
        // proposed = denied > 0 makes `looks_ungrounded` return true:
        // every proposed call was rejected by the permission hook.
        let mut denied_tools = BTreeMap::new();
        denied_tools.insert("Bash".to_owned(), 2);
        TurnActivity {
            proposed: 2,
            completed: 0,
            failed: 2,
            denied: 2,
            tools: BTreeMap::new(),
            denied_tools,
        }
    }

    fn ctx<'a>(
        current_role: &'a str,
        captured: &'a CapturedTurn,
        known: &'a [&'a str],
        thread: &'a ThreadView,
    ) -> TurnContext<'a> {
        TurnContext {
            current_role,
            captured,
            known_roles: known,
            thread,
        }
    }

    #[test]
    fn continue_outcome_fans_out_filtered_mentions() {
        let cap = captured(
            "Will check with @security and @frontend.",
            vec!["security", "frontend"],
            TurnOutcome::Continue,
        );
        let thread = ThreadView::default();
        let known = ["host", "backend", "security", "frontend"];
        let decision = DefaultPolicy.route_after_turn(&ctx("backend", &cap, &known, &thread));
        assert_eq!(decision.targets, vec!["security", "frontend"]);
        assert!(decision.skip_note.is_none());
    }

    #[test]
    fn no_increment_drops_all_mentions_with_note() {
        let cap = captured(
            "@host: not in my lens.",
            vec!["host"],
            TurnOutcome::NoIncrement,
        );
        let thread = ThreadView::default();
        let known = ["host", "security"];
        let decision = DefaultPolicy.route_after_turn(&ctx("security", &cap, &known, &thread));
        assert!(decision.targets.is_empty());
        let note = decision.skip_note.expect("skip_note set for mentioned reply");
        assert!(note.contains("no_increment"));
        assert!(note.contains("@security"));
    }

    #[test]
    fn converged_silences_chain_when_no_mentions() {
        // A clean synthesis with no @-mention shouldn't produce a
        // user-visible note — the conversation just ends.
        let cap = captured(
            "Final answer below.",
            vec![],
            TurnOutcome::Converged,
        );
        let thread = ThreadView::default();
        let known = ["host", "backend"];
        let decision = DefaultPolicy.route_after_turn(&ctx("host", &cap, &known, &thread));
        assert!(decision.targets.is_empty());
        assert!(
            decision.skip_note.is_none(),
            "no note for a mention-less convergence"
        );
    }

    #[test]
    fn needs_user_halts_chain_with_note_when_mentions_present() {
        let cap = captured(
            "@host: budget question — user must decide.",
            vec!["host"],
            TurnOutcome::NeedsUser,
        );
        let thread = ThreadView::default();
        let known = ["host", "backend"];
        let decision = DefaultPolicy.route_after_turn(&ctx("backend", &cap, &known, &thread));
        assert!(decision.targets.is_empty());
        let note = decision.skip_note.expect("note set on needs_user");
        assert!(note.contains("needs_user"));
    }

    #[test]
    fn grounding_gate_wins_over_outcome_claim() {
        // A role whose every tool call was denied may still produce a
        // `cr-status: converged` line — but its convergence claim is a
        // guess. Grounding gate runs first; outcome is irrelevant.
        let mut cap = captured(
            "I read the repo and we're done.\n@host",
            vec!["host"],
            TurnOutcome::Converged,
        );
        cap.activity = ungrounded_activity();
        let thread = ThreadView::default();
        let known = ["host", "backend"];
        let decision = DefaultPolicy.route_after_turn(&ctx("backend", &cap, &known, &thread));
        assert!(decision.targets.is_empty());
        let note = decision.skip_note.expect("ungrounded reply must explain itself");
        assert!(note.contains("permission denial"));
        // Outcome label should NOT appear — gate takes precedence.
        assert!(!note.contains("converged"));
    }

    #[test]
    fn self_mention_and_unknown_role_are_dropped() {
        let cap = captured(
            "Looking inward @backend and outward @ghost; reaching @security.",
            vec!["backend", "ghost", "security"],
            TurnOutcome::Continue,
        );
        let thread = ThreadView::default();
        let known = ["host", "backend", "security"];
        let decision = DefaultPolicy.route_after_turn(&ctx("backend", &cap, &known, &thread));
        // @backend (self) and @ghost (unknown) dropped; @security stays.
        assert_eq!(decision.targets, vec!["security"]);
    }

    #[test]
    fn duplicate_mentions_within_one_reply_are_preserved() {
        // A-005 locks in this behaviour: repeated `@peer` in one reply
        // produces multiple follow-up turns. Default policy must keep
        // it; if a future policy wants to dedup, that's an explicit
        // policy choice, not a default.
        let cap = captured(
            "@security one ask. @security second distinct ask.",
            vec!["security", "security"],
            TurnOutcome::Continue,
        );
        let thread = ThreadView::default();
        let known = ["host", "backend", "security"];
        let decision = DefaultPolicy.route_after_turn(&ctx("backend", &cap, &known, &thread));
        assert_eq!(decision.targets, vec!["security", "security"]);
    }

    #[test]
    fn outcome_short_circuit_skips_filtering_entirely() {
        // Even if the reply contains a self-mention or unknown role,
        // outcome-terminated replies short-circuit before the filter —
        // we want the same "chain ends here" signal regardless of
        // mention shape.
        let cap = captured(
            "@backend (self) @ghost",
            vec!["backend", "ghost"],
            TurnOutcome::Converged,
        );
        let thread = ThreadView::default();
        let known = ["host", "backend", "security"];
        let decision = DefaultPolicy.route_after_turn(&ctx("backend", &cap, &known, &thread));
        assert!(decision.targets.is_empty());
        assert!(decision.skip_note.is_some());
    }

    #[test]
    fn ungrounded_without_mentions_stops_silently() {
        // Grounding gate's user-visible hint only fires when the reply
        // also contained `@`-mentions (otherwise there is nothing to
        // explain skipping). Lock the silent-stop path so a refactor
        // doesn't start spamming a hint on ordinary mention-less turns
        // whose tools happened to fail.
        let mut cap = captured("Nothing routable here.", vec![], TurnOutcome::Continue);
        cap.activity = ungrounded_activity();
        let thread = ThreadView::default();
        let known = ["host", "backend"];
        let decision = DefaultPolicy.route_after_turn(&ctx("backend", &cap, &known, &thread));
        assert!(decision.targets.is_empty());
        assert!(
            decision.skip_note.is_none(),
            "no mentions → no skip note, even when ungrounded"
        );
    }

    #[test]
    fn grounding_gate_runs_strictly_before_outcome_check() {
        // The "gate first" invariant is the load-bearing part of the
        // decision order: a role whose tools were systematically denied
        // can still emit any `cr-status:` it likes, but its outcome
        // claim is a guess. We already test the user-visible note in
        // `grounding_gate_wins_over_outcome_claim`; this isolates the
        // *property* that the gate decided, not the outcome path.
        for outcome in [
            TurnOutcome::NoIncrement,
            TurnOutcome::Converged,
            TurnOutcome::NeedsUser,
        ] {
            let mut cap = captured("Claim with @host", vec!["host"], outcome);
            cap.activity = ungrounded_activity();
            let thread = ThreadView::default();
            let known = ["host", "backend"];
            let decision = DefaultPolicy.route_after_turn(&ctx("backend", &cap, &known, &thread));
            assert!(decision.targets.is_empty(), "outcome {outcome:?}");
            let note = decision
                .skip_note
                .as_deref()
                .expect("gate must explain skip for any outcome");
            // Gate's diagnostic identifies the permission failure, not
            // the outcome variant — proves the gate fired first.
            assert!(
                note.contains("permission denial") || note.contains("tool calls failed"),
                "outcome {outcome:?}: gate diagnostic missing in note: {note}"
            );
            for label in ["no_increment", "converged", "needs_user"] {
                assert!(
                    !note.contains(label),
                    "outcome {outcome:?}: gate skip-note must not echo outcome label {label}"
                );
            }
        }
    }

    #[test]
    fn empty_known_roles_drops_every_mention() {
        // When no roles are running (or all crashed mid-chain), every
        // mention is "unknown" by definition. The filter must drop
        // them all; the chain ends cleanly without dispatching to
        // nobody.
        let cap = captured(
            "Routing to @host and @security.",
            vec!["host", "security"],
            TurnOutcome::Continue,
        );
        let thread = ThreadView::default();
        let known: [&str; 0] = [];
        let decision = DefaultPolicy.route_after_turn(&ctx("backend", &cap, &known, &thread));
        assert!(decision.targets.is_empty());
        // Continue + filter-only-empty is a silent stop, like today.
        assert!(decision.skip_note.is_none());
    }
}
