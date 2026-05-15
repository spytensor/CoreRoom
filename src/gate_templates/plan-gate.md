# Plan Gate

Create a plan before Tier 1 implementation.

Required structure:

```text
# Plan: <feature>

## Scope
- In:
- Out:

## Files and Impact
- <path>:<line> - expected change

## Rollback Strategy
- ...

## Sign-off Checklist
| ID | Predicate | Verification Method | Pass Criterion |
| --- | --- | --- | --- |
| SO-1 | ... | ... | ... |
```

Record it:

```bash
cr gate artifact --thread <thread_id> --kind plan --path <artifact-path> \
  --role <role> --turn <turn_id>
```
