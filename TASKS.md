# CoreRoom Milestone Ledger

## Current Truth

- Latest aligned release tag: `v0.9.19`.
- Active milestone/tracker for autonomous pickup: none. `AGENTS.md` is the
  controlling worker protocol and must be updated before starting a new active
  milestone or tracker.
- GitHub issue state at the 2026-05-27 truth-alignment sweep: no open issues
  returned by `gh issue list --state open`.
- Commands remain automation, CI, debug, and recovery surface. The happy path is
  user intent -> `@host` -> role/gate/evidence/tracker orchestration.

## Completed Bootstrap Milestones

### v0.6 Engineering Control Room Bootstrap

- [x] Pin GitHub epic #202 for v0.6.
- [x] Create v0.6 tracker and implementation issues (#202-#212).
- [x] Create v0.7 backlog tracker and issues (#213-#218).
- [x] Update GitHub repository description and topics toward Engineering Control Room positioning.
- [x] #203: Land A-016 naming/product-positioning amendment.
- [x] #204: Land A-017 host authority and host-led engineering control protocol.
- [x] #205: Add AGENTS.md as the external AI worker protocol.
- [x] #206: Implement host intent classification.
- [x] #207: Define WorkOrder model and GitHub binding.
- [x] #208: Define project Source Registry.
- [x] #209: Define WorkOrder-scoped ContextPack.
- [x] #210: Define Evidence Packet model.
- [x] #211: Enforce tracker update protocol and PR evidence template.
- [x] #212: Add end-to-end host-led dogfood validation.

### v0.7 GitHub-native Engineering Loop / CoreRoom Rename

- [x] Pin GitHub epic #213 for v0.7.
- [x] #214: Implement GitHub-native WorkOrder/Issue/PR/CI status sync.
- [x] #215: Implement host-managed worker action layer.
- [x] #216: Implement multi-repo source graph and remote snapshots.
- [x] #217: Implement release readiness and project status rollup.
- [x] #218: Implement CoreRoom rename and compatibility migration.

### v0.8+ Console / Live Room Milestones

- [x] #238: v0.8 Console Data Plane tracker closed with dogfood evidence.
- [x] v0.9 Full-screen CoreRoom Console release path landed; see
  `docs/v0.9-real-user-dogfood.md` and `CHANGELOG.md`.
- [x] #377 and follow-up v0.10 work-status/chat-stream surface issues closed;
  see `docs/v0.10-chat-stream-vs-dashboard.md` and `CHANGELOG.md`.

## Maintenance Rule

This file is a human-readable ledger, not the active work queue. Before an AI
worker claims or starts new work, it must use `AGENTS.md` plus fresh GitHub
state as the authority. If this file, `AGENTS.md`, and GitHub disagree, fix the
truth-alignment drift before claiming product work is in scope.
