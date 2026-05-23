# CoreRoom rename and compatibility plan

Issue: #218  
Tracker: #213  
Decision date: 2026-05-23

## Decision

The product name is **CoreRoom**.

The searchable descriptor is **Engineering Control Room for AI Agents**.

The short positioning statement is:

> CoreRoom is the host-led Engineering Control Room for AI-assisted software
> engineering change.

The `cr` command remains the stable happy-path command. It maps cleanly to
CoreRoom and avoids unnecessary workflow churn. The optional long alias is
`coreroom`; `croom` is a legacy compatibility alias during the rename window.

## Surface policy

| Surface | v0.7 decision | Compatibility policy |
| --- | --- | --- |
| Product name | CoreRoom | Active docs, templates, runtime help, and splash text use CoreRoom. |
| Product descriptor | Engineering Control Room for AI Agents | Use in README, npm metadata, Cargo description, tracker language, and repo metadata. |
| Repository target | `spytensor/CoreRoom` preferred; `spytensor/coreroom` acceptable if owner prefers lowercase URLs | Owner-only GitHub rename happens after the staged PR lands. Existing `spytensor/codeRoom` URLs remain valid until owner action. |
| Primary command | `cr` | Stable and non-breaking. |
| Long command alias | `coreroom` | Added to npm metadata where packaging supports it. |
| Legacy command alias | `croom` | Kept as legacy compatibility for one rename window. |
| Rust package/lib | `coderoom` retained in v0.7 | Full crate rename to `coreroom` is deferred because it touches every integration import and release consumer. |
| npm package | `@spytensor/coreroom` target | `@spytensor/coderoom` remains the legacy package spelling and should publish a compatibility/deprecation note if already public. |
| Project state dir | Future default `.coreroom`; legacy `.coderoom` | v0.7 adds tested resolution policy. Write-path migration remains explicit and must not auto-move user files silently. |
| Env vars | `COREROOM_*` preferred; `CODEROOM_*` legacy | New spelling wins when both are set. Legacy spelling remains accepted during the compatibility window. |
| CREP | CoreRoom Event Protocol | Acronym remains stable. |
| Control blocks | `cr-task` retained | `cr` still maps to CoreRoom; no protocol churn in v0.7. |
| Historical docs/changelog | Preserve old references when they describe past releases | Active docs should use CoreRoom. Historical release entries may say CodeRoom with this rename note as context. |

## Audit

Command used:

```bash
rg --count-matches "CodeRoom|codeRoom|coderoom|CODEROOM|\\.coderoom|@spytensor/coderoom|croom|github\\.com/spytensor/codeRoom|spytensor/codeRoom" -S .
```

The audit found references across active docs, runtime help, package metadata,
tests/fixtures, source comments, historical docs, and generated hook internals.
v0.7 handles them by category:

| Category | Decision |
| --- | --- |
| Active product docs | Migrate to CoreRoom where user-facing. |
| Worker protocol | Migrate to CoreRoom and v0.7 tracker #213. |
| PR template | Migrate tracker language to v0.7/#213. |
| Cargo/npm metadata | Migrate descriptions and npm target package; keep Rust crate name in v0.7. |
| Runtime help/splash/setup text | Migrate visible product text to CoreRoom. |
| `.coderoom` paths | Keep runtime write path for v0.7; add tested `.coreroom` resolution policy. |
| `CODEROOM_*` env vars | Add `COREROOM_*` alias policy and compatibility tests. |
| Tests/fixtures | Update snapshots only where visible product text changed. Keep path fixtures for legacy compatibility. |
| Hook sentinels and generated protocol files | Keep legacy identifiers until a dedicated hook migration issue. |
| Historical changelog and old architecture docs | Keep historical references unless they are active entry points. |

## State directory migration

v0.7 does not silently move `.coderoom/` to `.coreroom/`.

The accepted resolution policy is:

- If only `.coreroom/` exists, use it.
- If only `.coderoom/` exists, treat it as legacy-compatible and warn that
  migration requires explicit confirmation.
- If neither exists, future new-project initialization should prefer
  `.coreroom/` once the write-path migration is enabled.
- If both exist, fail loudly and ask the user to resolve the conflict.

This policy is implemented in `coderoom::rename::resolve_state_dir` and covered
by `tests/rename_migration_test.rs`.

The actual write-path migration is deferred intentionally. It should be a
separate issue because it changes init behavior, hook templates, session paths,
lock files, and restore/rollback expectations.

## Environment-variable migration

Preferred names use `COREROOM_*`.

Legacy `CODEROOM_*` names remain accepted during the compatibility window. When
both are set, the `COREROOM_*` spelling wins.

`COREROOM_NO_UPDATE_CHECK` and legacy `CODEROOM_NO_UPDATE_CHECK` are accepted
by the update notifier. Alias resolution is tested without mutating process
environment.

## GitHub owner-only steps

After this PR lands, the owner should update repository metadata:

- Rename repo to `spytensor/CoreRoom` or `spytensor/coreroom`.
- Set description to:
  `CoreRoom is the Engineering Control Room for AI Agents: a host-led, GitHub-gated system for AI-assisted software engineering change.`
- Add/keep topics:
  `coreroom`, `engineering-control-room`, `ai-agents`, `agentic-engineering`,
  `github-issues`, `work-orders`, `ai-engineering`, `codex`, `claude-code`.
- Confirm GitHub redirects from `spytensor/codeRoom`.
- Update release badges/URLs after the repo rename is complete.

## Release notes

CoreRoom v0.7 starts the staged rename from CodeRoom to CoreRoom:

- Product-facing docs and runtime help now use CoreRoom.
- `cr` remains the primary command.
- npm metadata targets `@spytensor/coreroom` with `coreroom` as a long-form
  alias and `croom` as a legacy alias.
- Rust package/lib name remains `coderoom` for this stage.
- `.coderoom/` remains supported; `.coreroom/` migration is defined but not
  silently executed.
- `COREROOM_NO_UPDATE_CHECK` is preferred; `CODEROOM_NO_UPDATE_CHECK` remains
  accepted.

## Rollback

Revert the rename PR. Because `cr` stays stable and `.coderoom/` is not moved,
rollback does not require filesystem migration.

If `@spytensor/coreroom` is published before rollback, keep the package with a
compatibility/deprecation note instead of deleting it abruptly. If the GitHub
repo is renamed, GitHub redirects old URLs; docs can be reverted in a follow-up
PR while the owner decides whether to rename the repo back.
