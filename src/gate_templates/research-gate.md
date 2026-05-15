# Research Gate

Produce code-first research before implementation.

Required structure:

```text
# Research: <feature>

## Source Evidence
- <path>:<line> - what this proves

## Data Flow
1. Entry point:
2. State or IO boundary:
3. Downstream effect:

## Edge Cases
- ...

## Open Questions
- ...
```

Record it:

```bash
cr gate artifact --thread <thread_id> --kind research --path <artifact-path> \
  --role <role> --turn <turn_id>
```
