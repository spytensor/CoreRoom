# Code Review Gate

Strict review prompt for Tier 1 work.

Required output fields:

```text
reviewer_role: <role>
engine: <cc|codex|gemini>
model: <model>
blocking_count: <n>
warning_count: <n>
cross_model_satisfied: true|false
all_blockings_resolved: true|false
```

Review dimensions:

- Correctness against the plan and acceptance criteria.
- Data-flow safety across entry points, state, IO, and downstream effects.
- Cross-path behavior and edge cases.
- Error handling and rollback behavior.
- Plan/diff drift.
- Verification evidence.
- Consistency grep for nearby patterns.

Every finding that claims code evidence must cite `path:line`.

Record the review:

```bash
cr gate reviewer --thread <thread_id> --role <reviewer-role> \
  --engine <cc|codex|gemini> --model "<model>" --turn <turn_id> \
  --blocking-count <n> --warning-count <n> --file-line-evidence \
  --all-blockings-resolved --artifact <artifact-path>
```
