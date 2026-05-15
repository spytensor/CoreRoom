# Plan Review Gate

Review a Tier 1 plan before implementation.

Required checks:

- The plan cites source files with `path:line` evidence.
- The data-flow trace reaches the actual boundary being changed.
- Edge cases and rollback are concrete.
- The `Sign-off Checklist` has `SO-N` rows with predicate, method, and pass criterion.
- The plan avoids expanding scope beyond the user request.

Output findings with severity and file:line evidence. Do not approve
correctness; report whether the plan evidence is structurally complete.
