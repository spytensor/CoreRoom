# Sign-off Gate

Map final evidence back to every `SO-N` row from the plan.

Required structure:

```text
# Sign-off: <feature>

| ID | Evidence | Result |
| --- | --- | --- |
| SO-1 | <command output, path:line citation, or manual check result> | pass|fail |
```

Record it:

```bash
cr gate artifact --thread <thread_id> --kind signoff --path <artifact-path> \
  --role <role> --turn <turn_id>
```
