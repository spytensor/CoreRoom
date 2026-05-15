# Tier Classification Gate

Use this when a host receives a code-changing request.

Output:

```text
Tier: 0 | 1
Reason:
- ...
Gate ledger:
- thread_id: ...
- feature: ...
```

Tier 0 is for read-only review or tiny, low-risk edits where inline evidence
and lightweight checks are enough. Do not write `.coderoom/` gate or review
artifacts for Tier 0 unless the user explicitly asks for a ledger.
Tier 1 is for user-visible behavior, data flow, security, migrations,
release behavior, broad refactors, or any change where a missed edge case
would be expensive.

For Tier 1, initialize a ledger:

```bash
cr gate init --thread <thread_id> --tier 1 --feature "<work item>" \
  --role <implementer-role> --engine <cc|codex|gemini> --model "<model>" \
  --turn <turn_id>
```
