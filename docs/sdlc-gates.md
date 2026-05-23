# SDLC Gates

CodeRoom's SDLC gate support is host-first. Users can ask for work normally;
the host role is expected to classify the work, initialize a gate when needed,
delegate review, and close the gate before claiming completion.

Starting with A-017, this is part of the wider host-led engineering control
protocol. `@host` is the highest in-room authority for project-level workflow:
it owns intake, WorkOrder proposal, source/context discovery, delegation, gate
progression, evidence collection, tracker updates, and final status summary.
The user remains the final owner. Specialist roles may advise or block within
declared authority scopes, but they do not bypass `@host` for gate closure or
completion claims.

Gate evidence is structural, not semantic approval. For the trust boundaries
that reviews must preserve, see `docs/threat-model.md`.

## Files

- `.coderoom/gates/<thread-id>.json` stores one ledger per work thread.
- `.coderoom/gates/active` points at the most recently touched ledger.
- `.coderoom/gates/<thread-id>/<phase>.md` stores the structured notes for
  each phase the thread enters.
- `.coderoom/gates/<thread-id>/reviews/<role>.toml` stores binding
  authority-scoped plan review decisions for the current plan SHA.
- `.coderoom/gate-templates/*.md` stores reusable gate prompts.

Ledgers are structural evidence. They do not approve correctness.

## Host Confirmation Boundary

The happy path is conversational, but persistent state changes still need
explicit confirmation. `@host` must ask before creating or binding GitHub
Issues, updating milestone trackers, registering or refreshing project sources,
overriding role vetoes, moving Tier 1 work into implementation, preparing PR
completion claims, or claiming release readiness.

`@host` may classify work, summarize status, suggest roles, draft WorkOrders,
inspect local state, and report missing evidence without confirmation.

## Tier 0 / Read-Only Boundary

Tier 0 covers read-only reviews and tiny, low-risk edits where an inline
answer plus lightweight checks is enough. For read-only review, CodeRoom roles
may inspect repository files, docs, config, tests, local logs, and command
output needed to cite evidence. They must not mutate project files, write
`.coderoom/` review evidence, or append gate artifacts, reviewers, or
verification records unless the user explicitly asks for a persistent gate
ledger.

Tier 0 output should be reported inline with `path:line` citations and commands
inspected. A `cr gate init --tier 0 ...` command is an explicit ledger write,
but Tier 0 ledgers reject later `artifact`, `reviewer`, and `verify` evidence
writes. Re-run as Tier 1 when persistent evidence, cross-model review, or
release sign-off is needed.

## Typical Tier 1 Flow

```bash
cr gate init --thread <thread_id> --tier 1 --feature "short title" \
  --role host --engine cc --model "claude-sonnet-4" --turn <turn_id>

cr gate phase <thread_id> discovery
cr gate artifact --thread <thread_id> --kind discovery --path docs/gates/discovery.md

cr gate phase <thread_id> plan
# Fill `.coderoom/gates/<thread_id>/plan.md` with frontmatter:
# ---
# scopes: [infra, deployment]
# ---
cr gate artifact --thread <thread_id> --kind plan --path .coderoom/gates/<thread_id>/plan.md

cr gate phase <thread_id> review
cr gate role-review <thread_id> sre approve
cr gate role-review <thread_id> release approve
cr gate reviewer --thread <thread_id> --role security --engine codex \
  --model "gpt-5" --turn <turn_id> --blocking-count 0 --warning-count 1 \
  --file-line-evidence --all-blockings-resolved --artifact docs/gates/review.md

cr gate phase <thread_id> signoff
cr gate artifact --thread <thread_id> --kind signoff --path docs/gates/signoff.md

cr gate phase <thread_id> implement
cr gate phase <thread_id> qa
cr gate verify --thread <thread_id> --command "cargo test --all-features --locked" \
  --ok --evidence "test result: ok. 42 passed; 0 failed"

cr gate close --thread <thread_id>
```

If `close` blocks, CodeRoom prints actionable missing evidence. A bypass is
explicit and recorded:

```bash
cr gate close --thread <thread_id> --bypass "User accepted missing second reviewer for emergency fix."
```

Authority-scoped plan vetoes have a narrower override command. Use it only
when a configured authority role rejects the current plan SHA and the user
explicitly accepts the risk:

```bash
cr gate override <thread_id> --role security --reason "Emergency patch; rollback plan accepted."
```

## Tier 1 Structural Rules

- Discovery, plan, review, and sign-off artifacts must be recorded.
- Plan artifacts must include a `Sign-off Checklist` with `SO-N` rows.
- `.coderoom/gates/<thread-id>/plan.md` must declare frontmatter scopes
  before `plan -> review`; accepted scopes are `deployment`, `infra`,
  `secrets`, `data-policy`, `compliance`, and `dependencies`.
- `review -> signoff` requires every role whose configured `authority`
  intersects the plan scopes to approve the current plan SHA.
- If no configured role matches any plan scope, CodeRoom warns but does not
  block. If at least one role matches but some plan scopes remain uncovered,
  sign-off advancement is blocked.
- Changing the plan artifact after approval makes the prior authority review
  stale. A `reject` or `needs-revision` decision blocks sign-off until the role
  approves the current SHA or the user records an override.
- Review artifacts must include reviewer role, engine, model, finding counts,
  `cross_model_satisfied`, and `all_blockings_resolved`.
- Review findings that claim code evidence must cite `path:line`.
- At least two reviewer turns are required.
- At least one independent reviewer must be from a different model family than
  the implementer.
- Verification evidence must include real command output or cited evidence.

Tier 0 gates skip these structural requirements and cannot record hidden
evidence writes. Routing, permission, resume, and reviewer-provenance changes
should also satisfy the review checklist in `docs/threat-model.md`.

## Phase Workflow

Gate phases are linear:

```text
intake -> discovery -> plan -> review -> signoff -> implement -> qa -> closed
```

`rejected` is a terminal branch from `review` or `signoff`. Use
`cr gate phase <thread_id> <next-phase>` for normal advancement. Skips and
regressions are rejected; rollback requires `--rollback "<reason>"` and can
only target an earlier linear phase.

Roles can block the active phase by ending their reply with:

```text
cr-phase-block: <reason>
```

The marker must be the final line. CodeRoom strips it from the visible reply,
records a `PhaseBlocked` event in `.coderoom/messages.jsonl`, appends the block
to the gate ledger, and suppresses follow-up auto-routing for that turn.
