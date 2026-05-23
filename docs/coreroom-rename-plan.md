# CoreRoom v0.7 release rename plan

Issue: #218  
Tracker: #213  
Release date: 2026-05-23

## Decision

The product name is **CoreRoom**.

The searchable descriptor is **Engineering Control Room for AI Agents**.

The short positioning statement is:

> CoreRoom is the host-led Engineering Control Room for AI-assisted software
> engineering change.

The `cr` command remains the stable happy-path command. The optional long-form
alias is `coreroom`.

## Final v0.7 surface policy

| Surface | v0.7 decision |
| --- | --- |
| Product name | CoreRoom |
| Product descriptor | Engineering Control Room for AI Agents |
| Repository | `spytensor/CoreRoom` |
| Primary command | `cr` |
| Long command alias | `coreroom` |
| Rust package/lib | `coreroom` |
| npm package | `@spytensor/coreroom` |
| Project state dir | `.coreroom` |
| Environment variables | `COREROOM_*` |
| CREP | CoreRoom Event Protocol |
| Control blocks | `cr-task` retained because `cr` remains the product command |

## Release audit

Before tagging v0.7.0, run the release rename audit from the PR evidence. The
expected result is no matches for retired pre-v0.7 spellings.

Then verify the active release surfaces:

```bash
cargo check --all-targets
cargo test --all-features --locked
cargo clippy --all-targets --all-features -- -D warnings
npm --prefix npm pack --dry-run --json
```

## NPM release policy

The npm package is `@spytensor/coreroom`.

The package exposes:

- `cr` as the primary command
- `coreroom` as the long-form command alias

The installer downloads artifacts from
`https://github.com/spytensor/CoreRoom/releases`.

The release workflow stages only the supported binary names for v0.7:

- `cr`
- `coreroom`

## State and config policy

New project state uses `.coreroom/`.

User config/cache paths use `coreroom` where platform conventions require a
directory name.

Environment variables use the `COREROOM_*` prefix.

The v0.7 rename does not silently move existing local state. Any later migration
for older local workspaces must be explicit, user-approved, and tracked as a
separate issue because it can affect sessions, locks, hook files, and rollback
expectations.

## GitHub owner steps

The repository is now `spytensor/CoreRoom`.

Repository metadata should stay aligned with:

- Description:
  `CoreRoom is the Engineering Control Room for AI Agents: a host-led, GitHub-gated system for AI-assisted software engineering change.`
- Topics:
  `coreroom`, `engineering-control-room`, `ai-agents`, `agentic-engineering`,
  `github-issues`, `work-orders`, `ai-engineering`, `codex`, `claude-code`.

## Release notes

CoreRoom v0.7.0 completes the rename across active product surfaces:

- Product-facing docs and runtime help use CoreRoom.
- Repository URLs target `spytensor/CoreRoom`.
- Cargo metadata uses package/lib name `coreroom`.
- npm metadata uses `@spytensor/coreroom`.
- Release artifacts expose `cr` and `coreroom`.
- Project state uses `.coreroom/`.
- Environment variables use `COREROOM_*`.

## Rollback

Rollback is a normal Git revert of the rename PR before v0.7.0 is tagged.

After the v0.7.0 tag is published, rollback requires a new corrective release
because package names, release artifacts, and public documentation will already
have been published under the CoreRoom surface.
