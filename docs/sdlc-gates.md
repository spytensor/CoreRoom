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

## Host Intent Classification

`@host` classifies every project-level request before creating persistent
state or delegating implementation. The classification output is structured:

```text
Classification: tier-0-inline | persistent-workorder | constitution-amendment | release-audit-review | insufficient-context
Reason:
- <why this category fits>
Next step:
- <inline answer, draft WorkOrder, ask confirmation, or stop>
Confirmation required: yes | no
```

Categories:

- `tier-0-inline`: read-only review, explanation, or tiny low-risk edit where
  inline evidence is enough and no `.coderoom/` ledger is needed.
- `persistent-workorder`: code, docs, workflow, or project work that needs a
  GitHub Issue, branch, PR, evidence, and tracker row.
- `constitution-amendment`: product/architecture/trust-boundary changes that
  must update `docs/proposed-amendments.md` before implementation.
- `release-audit-review`: release, compliance, incident, security, or audit
  work that needs fresh context, stronger evidence, and explicit signoff.
- `insufficient-context`: the host lacks enough facts to classify safely and
  must ask a narrow question or request the missing source.

Classification is not approval. It decides the workflow path. Persistent state
changes still follow the confirmation boundary above.

## WorkOrders

Starting in v0.6, persistent engineering work can be represented as a
WorkOrder under `.coderoom/work-orders/<id>.toml`. A WorkOrder is the
project-level binding record between host intent, GitHub Issue, SDLC gate,
branch, PR, tracker row, and evidence expectations. It does not replace the
GitHub Issue.

The persisted WorkOrder schema uses camelCase keys:

```toml
schemaVersion = 1
id = "WO-0207"
title = "WorkOrder model and GitHub binding"
objective = "Define a WorkOrder model and bind it to GitHub Issue #207."
githubIssue = 207
phase = "v0.6.0 - Engineering Control Room"
epic = "WorkOrder / GitHub Binding"
gateThread = "thread-207"
branch = "feat/v0.6-207-workorder-github-binding"
pullRequest = 223
status = "in-review"
trackerIssue = 202
trackerCheckbox = "#207 - WorkOrder model and GitHub binding"

acceptanceCriteria = [
  "Define WorkOrder fields.",
  "Bind existing GitHub Issue without mutating the issue body.",
]

requiredEvidence = [
  "changed-files",
  "validation",
  "risks",
  "rollback",
  "tracker-update",
]
```

Canonical status values are `draft`, `proposed`, `ready`, `in-progress`,
`in-review`, `merged`, `blocked`, and `closed`.

`@host` may draft a WorkOrder after classifying the request as
`persistent-workorder`. Creating or binding the GitHub Issue still requires
user confirmation. Binding an existing issue updates only the local WorkOrder;
it must not silently change the GitHub Issue body, labels, milestone, or
comments.

## Project Source Registry

Source Registry is the v0.6 project-level catalog of dependency context. It is
stored at `.coderoom/source-registry.toml` and is distinct from both role
knowledge and WorkOrder ContextPacks:

- Role knowledge is long-lived role-specific prompt material.
- Source Registry lists project sources with pins, trust, owners, visibility,
  purpose, and refresh policy.
- ContextPack, added later in v0.6, selects the minimal source slices for a
  specific WorkOrder.

The persisted Source Registry schema uses camelCase keys:

```toml
schemaVersion = 1

[[sources]]
id = "core-api"
kind = "local-repo"
path = "../core-api"
pin = "commit:0123456789abcdef"
trustLevel = "internal"
owner = "platform-team"
visibleRoles = ["host", "engineer"]
purpose = "Integration behavior and API contracts."
refreshPolicy = "on-confirmation"

[[sources]]
id = "security-policy"
kind = "policy-doc"
path = "docs/policies/security.md"
pin = "sha256:abc123"
trustLevel = "policy"
owner = "security"
visibleRoles = ["host", "security"]
purpose = "Security constraints for source handling."
refreshPolicy = "manual"

[[sources]]
id = "provider-docs"
kind = "url-snapshot"
url = "https://docs.example.test/api"
pin = "snapshot:deadbeef"
trustLevel = "external-doc"
owner = "host"
visibleRoles = ["host", "engineer"]
purpose = "External API reference snapshot."
refreshPolicy = "on-confirmation"
```

Supported source kinds are `project-file`, `local-repo`, `git-repo`,
`url-snapshot`, `policy-doc`, `api-spec`, and `design-reference`. Trust levels
are `project`, `internal`, `external-doc`, `policy`, `generated`, and
`untrusted`. Refresh policies are `never`, `manual`, and `on-confirmation`;
there is no silent auto-refresh policy in v0.6.

`@host` may propose source entries during intake, but registration or
re-pinning requires user confirmation. Missing local files, inaccessible local
repos, missing pins, invalid trust levels, and invalid refresh policies fail
loudly before registry writes.

## WorkOrder ContextPacks

ContextPacks are WorkOrder-scoped selections from Source Registry. They are
stored under `.coderoom/context-packs/<id>.toml` and exist to keep delegation
small and reproducible: `@engineer` and `@security` can receive different
slices of the same registered sources.

The persisted ContextPack schema uses camelCase keys:

```toml
schemaVersion = 1
id = "CTX-WO-0209"
workOrder = "WO-0209"

[[entries]]
sourceId = "core-api"
path = "src/contracts.rs"
sourcePin = "commit:abc123"
trustLevel = "internal"
reason = "Engineer needs API contract definitions."
targetRoles = ["engineer"]

[entries.range]
startLine = 10
endLine = 40

[[entries]]
sourceId = "security-policy"
path = "docs/policies/security.md"
sourcePin = "sha256:def456"
trustLevel = "policy"
reason = "Security needs policy constraints."
targetRoles = ["security"]
```

Each entry must reference a registered `sourceId`, include a path/range or
snapshot reference, explain why the slice is needed, declare target roles, and
copy the source pin and trust level from the Source Registry. If the registry
pin changes later, the ContextPack remains auditable but `@host` must surface a
stale-context warning before delegation. If a selected source is unpinned, that
also produces a warning.

Target roles must be included in the source's `visibleRoles`. No role receives
every source by default.

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
