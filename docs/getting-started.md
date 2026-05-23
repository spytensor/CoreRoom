# Getting Started

## Basic Setup

Run CoreRoom in a project directory:

```bash
cr init -y
cr start
```

`cr init` seeds `.coreroom/` with starter roles:

- `host`
- `engineer`
- `reviewer`

Use the team preset for a fuller virtual-team scaffold:

```bash
cr init -y --preset team
```

That adds `sre`, `security`, and `qa` role skeletons with empty
`knowledge/` directories and explicit empty authority declarations.

Re-running `cr init` is safe. Existing `.coreroom/` files are left untouched,
so a second run should produce no project diff.

## Claude Code Hooks

To let Claude Code tool calls pass through CoreRoom's PreToolUse permission
hook, opt in during init:

```bash
cr init -y --with-claude-hooks --preset team
```

This writes:

- `.coreroom/` starter roles and gate templates
- `.claude/settings.json` with a CoreRoom-aware `PreToolUse` hook
- `.claude/.coreroom-managed.json` as the generated-file marker

If `.claude/settings.json` already exists, CoreRoom parses and merges it. The
existing file is backed up as `.claude/settings.json.bak.<timestamp>` before
the merged settings are written. Existing non-CoreRoom hooks are preserved.

To re-apply the latest hook template later:

```bash
cr init --upgrade-hooks
```

The merge is idempotent: running the same command again does not create a new
backup or change the settings file when the CoreRoom hook is already current.
