# SDLC Gates

CodeRoom's SDLC gate support is host-first. Users can ask for work normally;
the host role is expected to classify the work, initialize a gate when needed,
delegate review, and close the gate before claiming completion.

## Files

- `.coderoom/gates/<thread-id>.json` stores one ledger per work thread.
- `.coderoom/gates/active` points at the most recently touched ledger.
- `.coderoom/gate-templates/*.md` stores reusable gate prompts.

Ledgers are structural evidence. They do not approve correctness.

## Typical Tier 1 Flow

```bash
cr gate init --thread <thread_id> --tier 1 --feature "short title" \
  --role host --engine cc --model "claude-sonnet-4" --turn <turn_id>

cr gate artifact --thread <thread_id> --kind research --path docs/gates/research.md
cr gate artifact --thread <thread_id> --kind plan --path docs/gates/plan.md

cr gate reviewer --thread <thread_id> --role security --engine codex \
  --model "gpt-5" --turn <turn_id> --blocking-count 0 --warning-count 1 \
  --file-line-evidence --all-blockings-resolved --artifact docs/gates/review.md

cr gate verify --thread <thread_id> --command "cargo test --all-features --locked" \
  --ok --evidence "test result: ok. 42 passed; 0 failed"

cr gate artifact --thread <thread_id> --kind signoff --path docs/gates/signoff.md
cr gate close --thread <thread_id>
```

If `close` blocks, CodeRoom prints actionable missing evidence. A bypass is
explicit and recorded:

```bash
cr gate close --thread <thread_id> --bypass "User accepted missing second reviewer for emergency fix."
```

## Tier 1 Structural Rules

- Research, plan, review, and sign-off artifacts must be recorded.
- Plan artifacts must include a `Sign-off Checklist` with `SO-N` rows.
- Review artifacts must include reviewer role, engine, model, finding counts,
  `cross_model_satisfied`, and `all_blockings_resolved`.
- Review findings that claim code evidence must cite `path:line`.
- At least two reviewer turns are required.
- At least one independent reviewer must be from a different model family than
  the implementer.
- Verification evidence must include real command output or cited evidence.

Tier 0 gates skip the cross-model review requirements.
