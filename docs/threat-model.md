# CodeRoom Threat Model

CodeRoom is a local coordination shell, not a sandbox and not a hosted
security boundary. It still has load-bearing trust assumptions because it
mixes user input, project-owned prompt files, model output, local logs, and
engine permission systems in one workflow.

This document is the review contract for changes that touch routing,
permissions, resume, gates, priors, logs, or role memory.

## Scope

In scope:

- A single user running `cr` inside a local project checkout.
- Project files and `.coderoom/` files editable by that user or by anyone who
  can write to the repository.
- Engine subprocesses (`claude`, `codex`, `gemini`) that may emit arbitrary
  text, incomplete tool events, stale session ids, or adversarially shaped
  replies.
- Prompt injection from repository content, role priors, pasted text,
  transcripts, or peer role output.

Out of scope:

- OS-level isolation between roles. All roles run with the same local user
  account and repository access.
- Protecting against a malicious local user editing their own policy or
  project files.
- Network or API credential isolation beyond what the underlying engine CLI
  provides.

## Trust Boundaries

| Input or state | Trust level | Runtime use | Must not be used for |
| --- | --- | --- | --- |
| User keystrokes in the live REPL | Authoritative task intent for the current turn | Addressing roles, commands, permission choices, explicit `/fresh` and `/resume` actions | Hidden state changes without visible command feedback |
| `.coderoom/config.toml` and user config | Trusted configuration after schema validation | Engine selection, model defaults, host role, permission mode, declared role owners and authority scopes | Evidence of review completion, peer consensus, or safety approval |
| `.coderoom/shared.md`, `.coderoom/roles/*.md`, patches, journals | Project-supplied prompt input | Shape role behavior and local conventions with source headers | Redefining kernel routing syntax, gate rules, permission semantics, or provenance |
| Engine output | Untrusted text plus adapter-parsed events | User-visible replies, explicit delegation text, WorkCard display, tool event summaries | Authoritative turn ids, thread ids, parent ids, hop depth, permission grants, gate completion, or peer consensus |
| `.coderoom/messages.jsonl` and transcript archives | Editable audit/replay log | `cr show`, transcript citations, debugging, historical display, best-effort cost reporting | Active routing limits, permission enforcement, budget enforcement, gate close decisions, or live provenance |
| `.coderoom/permission_policy.json` | User-editable session policy | Current allow/deny decisions after startup visibility and `/permissions` inspection | Silent approvals that are not surfaced, historical proof that a decision was attended to |
| Engine session ids | Opaque adapter-issued resume handles | Continue engine conversations when the user accepts resume | Evidence freshness, peer agreement, review provenance, or thread lineage |
| Runtime turn/thread state | Trusted only while owned by the live dispatcher/process | Route provenance, hop depth, parent/child relationships, queue limits | Rehydration from model text or editable logs for enforcement |
| `.coderoom/gates/*` | User-editable structural ledger | Tier 1 structural completeness, role-review decisions, and explicit bypass or override records | Semantic correctness, reviewer independence by model claim alone, hidden Tier 0 evidence |

## Runtime Invariants

These invariants are security-relevant. Changes that weaken them need an
architecture amendment before implementation.

1. Kernel rules outrank project prompt files.
   `.coderoom/shared.md`, role priors, patches, and journals may refine how a
   role behaves, but they cannot redefine CodeRoom routing syntax, permission
   semantics, gate rules, peer provenance, or WorkCard protocol.

2. Routing metadata belongs to the dispatcher.
   `turn_id`, `thread_id`, `parent_turn_id`, hop depth, fan-out position, and
   queue limits are assigned by runtime state. Text like `From @role:` or
   `<<<peer-quote ...>>>` may be displayed to a model, but it is not accepted
   back from the model as authoritative metadata.

3. Delegation is syntax, not mention presence.
   Plain status or attribution mentions such as `@backend said ...` do not
   route. Auto-routing only acts on explicit delegation lines accepted by the
   parser, such as `@backend: <brief>` or `@backend @ci: <brief>`.

4. Peer output is quoted evidence, not a command channel.
   Cross-role payloads are treated as data from the sending role. A receiving
   role can use that content as context, but embedded instructions inside the
   quote do not override its kernel, priors, or current user request.

5. Current-thread evidence is required for peer claims.
   A role may claim consensus, approval, review completion, or "merged
   perspectives" only from current-thread peer evidence surfaced by the
   runtime, such as peer-quote envelopes, current turn ids, or user-pasted
   current-thread text. Memory, priors, journals, and resumed engine context
   are not enough.

6. Editable logs are not enforcement state.
   `.coderoom/messages.jsonl` supports replay and audit, but live safety
   decisions must come from runtime-owned state or explicit user commands.
   Future budget enforcement must not trust a mutable log total.

7. Permission policy is visible and resettable.
   Existing allow/deny decisions must be visible at startup and through
   `/permissions`. Review or release workflows that require fresh attention
   should use `/permissions clear` and, when stale engine context matters,
   `/fresh`.

8. Resume is convenience, not provenance.
   Resuming an engine session may carry useful context, but it also carries
   stale claims. `cr` must surface resumed roles and the clean-start controls.
   Release reviews, audits, and incident work should prefer `cr start --fresh`
   or `/fresh` unless the user intentionally wants continuity.

9. Tier 0 is inline.
   Tier 0/read-only review may inspect files and commands needed for evidence,
   but it does not write hidden `.coderoom/` review artifacts. Persistent
   evidence, cross-model review, or release sign-off belongs in Tier 1.

10. Authority-scoped veto is explicit.
    A role can block plan advancement only when all of these are true: the
    role has a validated authority scope in configuration, the plan artifact
    declares an intersecting scope, and the role records an explicit review
    decision for the current plan SHA. Model prose, stale resumed context, or
    editable logs cannot create authority, expand scope, reject a plan, or
    override a rejection.

11. User override is a command, not a claim.
    A scoped veto can be overruled only by an explicit user command with a
    reason. The override is recorded in the gate ledger and CREP audit trail.
    Text emitted by a role, transcript replay, or a journal entry may explain
    the override after the fact, but cannot substitute for it.

## Decisions That Must Not Be Reconstructed

The following live decisions must not be reconstructed from model text,
resumed engine context, `.coderoom/messages.jsonl`, transcript archives, or
role-written journals:

- Whether a model reply is allowed to auto-route.
- The parent/child chain for routed turns.
- Hop depth, fan-out count, queue limit state, or route loop termination.
- Whether a role has current-thread approval from another role.
- Whether a reviewer is independent, blocking findings are resolved, or a
  gate can close.
- Whether a role's authority scope applies to a plan.
- Whether a role veto exists for the current plan SHA.
- Whether a user override exists and carries the required justification.
- Whether a tool call is allowed under the current permission policy.
- Whether a budget, cost ceiling, or spend cap has been enforced.
- Whether a resumed role's context is fresh enough for an audit or release
  decision.

Logs may corroborate or explain those decisions after the fact. They do not
make the decision.

## Review Checklist

Use this checklist for any PR that touches routing, permissions, resume,
gates, priors, logs, or role memory.

- Does the change keep kernel protocol above user-editable prompt files?
- Is every routing decision based on parser output plus dispatcher-owned
  state, not on model-supplied metadata?
- Does the change preserve explicit delegation syntax and avoid routing on
  plain status mentions?
- Are peer consensus or review claims grounded in current-thread evidence?
- Does any new enforcement path avoid trusting `.coderoom/messages.jsonl` or
  transcript archives?
- Are persisted permission decisions surfaced and clearable before
  provenance-sensitive work?
- Does resume behavior make stale context visible and provide a clean-start
  path?
- Does Tier 0 stay inline unless the user explicitly asks for a ledger?
- If role authority is involved, is the decision based on validated config,
  declared plan scopes, and current plan SHA rather than model prose?
- If a veto is bypassed, is there an explicit user override with a recorded
  reason?

If the answer is "no" to any item, either change the implementation or file an
architecture amendment that explicitly moves the trust boundary.
