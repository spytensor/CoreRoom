# CoreRoom

CoreRoom is the Engineering Control Room for AI Agents: a host-led,
GitHub-gated system for AI-assisted software engineering change.

```bash
npm install -g @spytensor/coreroom

cd your-project
cr
```

This package is a thin npm wrapper. On install it downloads the
matching pre-built `cr` binary from the
[GitHub Release](https://github.com/spytensor/codeRoom/releases) for
your platform and installs it. Supported: linux + macOS, x86_64 and
aarch64.

It also installs `coreroom` as a long-form alias and `croom` as a legacy
alias for the same binary in case `cr` conflicts with another command on
your PATH.

For the full project documentation, source, and architecture see
[github.com/spytensor/codeRoom](https://github.com/spytensor/codeRoom).
