# Pre-commit Gate

Before commit or completion claims, collect verification output.

Checklist:

- Format command output.
- Lint command output.
- Test command output.
- Any targeted manual or CLI check tied to `SO-N`.

Record each meaningful verification:

```bash
cr gate verify --thread <thread_id> --command "<command>" --ok \
  --evidence "<actual output or cited evidence>"
```

Then run:

```bash
cr gate validate --thread <thread_id>
cr gate close --thread <thread_id>
```

If close blocks, report the missing gate evidence instead of saying the
work is complete.
