<!--
PR title MUST follow Conventional Commits:
  feat: …       feat(adapter-cc): …
  fix: …        fix(repl): …
  chore: …      docs: …       refactor: …
  test: …       ci: …         perf: …

Bump scope must be a directory or module name (e.g. `adapter-cc`, `crep`,
`repl`, `bus`, `ci`, `docs`).
-->

## Summary

<!-- 1–3 sentences. What did you change and why? -->

## Architecture impact

- [ ] No change — purely follows `docs/architecture.md` as written.
- [ ] Refines an open question — links to its entry in `docs/proposed-amendments.md`.
- [ ] **Amends a locked decision** — must include the amendment in `docs/proposed-amendments.md` and reference it here. (This is rare; usually a separate PR is opened first that lands the amendment.)

## Linked issues

<!-- Closes #N / Refs #M / "none" if standalone -->

## Acceptance criteria

<!-- Copy the linked issue AC and tick only what this PR satisfies. -->

- [ ] AC-1:

## Test plan

- [ ] `cargo fmt --all -- --check` passes
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes
- [ ] `cargo test --all-features --locked` passes
- [ ] (if touching shell harness) `shellcheck spike/*.sh` passes
- [ ] Manual smoke (describe what you ran, against which engine):

## Evidence packet

- WorkOrder:
- Gate thread:
- Changed files:
- Commands run:
- Test/check results:
- Role reviews:
- Risks:
- Rollback:
- Unverified items:

## Tracker update

For v0.7 issues, the PR is incomplete until tracker #213 is updated.

- Tracker issue: #213
- Issue checkbox updated:
  - [ ] yes
  - [ ] no, reason:
- Milestone AC updated, if applicable:
  - [ ] yes
  - [ ] no, reason:
- Evidence Ledger row updated:
  - [ ] yes
  - [ ] no, reason:

If implementation is complete but tracker #213 is stale, report
`implementation complete, tracker incomplete`; do not claim `done`.

## Risk and rollback

- Risk:
- Rollback:

## Out of scope

<!-- What you intentionally didn't do, even though it's adjacent. Helps reviewers
     not ask why it's missing. -->
