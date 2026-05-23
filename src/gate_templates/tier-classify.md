# Host Intent Classification Gate

Use this when `@host` receives a project-level request. Classify before
creating persistent state, delegating implementation, or claiming completion.

Output:

```text
Classification: tier-0-inline | persistent-workorder | constitution-amendment | release-audit-review | insufficient-context
Reason:
- ...
Next step:
- ...
Confirmation required: yes | no
```

Categories:

- `tier-0-inline`: read-only review, explanation, or tiny low-risk edit where
  inline evidence and lightweight checks are enough.
- `persistent-workorder`: code, docs, workflow, or project work that needs a
  GitHub Issue, branch, PR, evidence, and tracker row.
- `constitution-amendment`: product/architecture/trust-boundary changes that
  must update `docs/proposed-amendments.md` before implementation.
- `release-audit-review`: release, compliance, incident, security, or audit
  work that needs fresh context, stronger evidence, and explicit signoff.
- `insufficient-context`: missing facts prevent safe classification; ask a
  narrow question or request the missing source.

Tier 0 stays inline. Do not write `.coderoom/` gate or review artifacts for
Tier 0 unless the user explicitly asks for a ledger. Persistent WorkOrder,
constitution, and release/audit paths need explicit confirmation before
state changes.

For persistent Tier 1 work after confirmation, initialize a ledger:

```bash
cr gate init --thread <thread_id> --tier 1 --feature "<work item>" \
  --role <implementer-role> --engine <cc|codex|gemini> --model "<model>" \
  --turn <turn_id>
```
