# CoreRoom AI Worker Protocol

This file is the entry point for external AI coding workers operating in this
repository, including Codex, Claude Code, and other terminal coding agents.

## Current Project Phase

Active milestone: v0.8.0 - CoreRoom Console Data Plane.

Primary tracker: #238.

v0.7 tracker #213 is complete. Do not work on v0.9+ issues unless the user
explicitly pulls one into the active milestone.

## Operating Model

CoreRoom is the Engineering Control Room for AI Agents: a host-led,
GitHub-gated system for AI-assisted software engineering change. The happy
path is:

```text
user intent -> @host -> scoped issue/work -> branch -> PR -> CI/evidence -> tracker
```

Commands are automation, CI, debug, and recovery surface. Do not make users
memorize command choreography as the product path.

## Issue Pickup Rules

Only pick up an issue when all are true:

- The issue has `status:ready`.
- The issue has `codex-ready`.
- The issue is not labelled `constitution`.
- The issue is not labelled `human-only`.
- The issue belongs to the active v0.8 milestone, unless the user explicitly
  re-scopes it.

If an issue is ambiguous, blocked, missing acceptance criteria, or conflicts
with this file, comment on the issue and stop. Do not guess.

## Branch and PR Discipline

- Use one branch per issue.
- For v0.8 issues, branch from `main` as
  `feat/v0.8-<issue-number>-<short-slug>`.
- Implement strictly against the issue Acceptance Criteria.
- Do not touch files outside the issue scope unless the PR explains why.
- Do not mix constitution decisions with implementation unless the issue
  explicitly allows it.

## Required PR Evidence

Every PR must include:

- Linked issue using `Closes #<issue>`.
- Checked acceptance criteria.
- Changed files summary.
- Validation commands and results.
- Evidence Packet or inline evidence summary.
- Snapshot/fixture evidence when console, status, view-model, transcript, or
  host output changes.
- Risks and remaining gaps.
- Rollback plan.
- Tracker update section.

## Tracker Rule

An issue is not done until the tracker is updated.

For v0.8, the completing PR must update #238 by:

- Ticking the issue checkbox.
- Updating any satisfied milestone acceptance criteria.
- Updating the Evidence Ledger row with PR link, validation evidence, changed
  files summary, and remaining risk.

If implementation is complete but the tracker is stale, report:

```text
implementation complete, tracker incomplete
```

Do not claim `done`.

`@host` must also detect stale tracker states before claiming completion:

- Issue closed but tracker checkbox unchecked.
- PR merged but Evidence Ledger row is not `merged` with Tracker Updated `yes`.
- Evidence Packet exists but the linked issue is not closed.

When stale, propose the exact tracker patch/update summary instead of closing
the work in prose.

## Host Authority

The user is the final owner. Inside CoreRoom, `@host` is the highest in-room
authority because it faces the user. Specialist roles may advise or block within
declared authority scopes, but they do not bypass `@host` for project-level
state, completion claims, or tracker closure.

## Validation Defaults

Use the narrowest meaningful checks for the change. Common commands:

```bash
git diff --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

For docs-only changes, `git diff --check` may be enough. For changes touching
templates, priors, init, role config, or gate behavior, run the relevant cargo
tests and explain why that scope is sufficient.

## Do Not

- Do not work on `constitution` or `human-only` issues unless the user directly
  instructs this specific worker in the current conversation.
- Do not pick up v0.9+ work while #238 is active.
- Do not infer completion from model prose.
- Do not update trackers without evidence.
- Do not claim a console, status panel, or dashboard state is valid unless it
  is derived from structural facts or an explicit fixture.
- Do not silently change package names, binary names, repo names, release
  scripts, or migration policy.
- Do not replace GitHub Issues, PRs, CI, or tracker evidence with chat history.
