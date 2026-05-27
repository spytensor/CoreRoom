# CoreRoom AI Worker Protocol

This file is the entry point for external AI coding workers operating in this
repository, including Codex, Claude Code, and other terminal coding agents.

## Current Project Phase

Latest release tag: v0.9.19.

Active milestone: none. All currently scoped GitHub issues were closed when
this file was last aligned (2026-05-27). Do not autonomously pick up new
milestone work until this section names an active milestone/tracker again, or
until the user explicitly scopes a specific issue/work item in the current
conversation.

Primary tracker: none.

Before issue pickup, refresh GitHub state. If GitHub state disagrees with this
section, treat this file as stale: propose the exact `AGENTS.md` update before
implementing anything beyond that alignment.

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
- The issue belongs to the active milestone/tracker named above, unless the
  user explicitly scopes it in the current conversation.
- If `Active milestone` is `none`, autonomous issue pickup is disabled; only
  direct user-scoped maintenance work may proceed.

If an issue is ambiguous, blocked, missing acceptance criteria, or conflicts
with this file, comment on the issue and stop. Do not guess.

## Branch and PR Discipline

- Use one branch per issue or direct user-scoped maintenance task.
- Fetch first and branch from current `main`.
- For issue work, use `feat/v<major.minor>-<issue-number>-<short-slug>` or
  `fix/v<major.minor>-<issue-number>-<short-slug>` as appropriate.
- For standalone maintenance explicitly scoped by the user, use
  `chore/<short-slug>` or `fix/<short-slug>`.
- Implement strictly against the issue Acceptance Criteria.
- Do not touch files outside the issue scope unless the PR explains why.
- Do not mix constitution decisions with implementation unless the issue
  explicitly allows it.

## Required PR Evidence

Every PR must include:

- Linked issue using `Closes #<issue>`, or `none` with the direct user-scoped
  reason for standalone maintenance.
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

An issue is not done until the tracker named above is updated. If
`Primary tracker` is `none`, the PR must say tracker update is not applicable
and explain the direct user-scoped reason.

When a primary tracker exists, the completing PR must update it by:

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
- Do not pick up future milestone work while `Active milestone` is `none`,
  unless the user directly scopes that work in the current conversation.
- Do not infer completion from model prose.
- Do not update trackers without evidence.
- Do not claim a console, status panel, or dashboard state is valid unless it
  is derived from structural facts or an explicit fixture.
- Do not silently change package names, binary names, repo names, release
  scripts, or migration policy.
- Do not replace GitHub Issues, PRs, CI, or tracker evidence with chat history.
