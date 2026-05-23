# CoreRoom Engineering Control Room Bootstrap Tasks

## Goal

Bootstrap CoreRoom as the Engineering Control Room for AI Agents: host-led
AI-assisted software engineering control with GitHub issue discipline,
dependency context, evidence packets, status rollups, and tracker closure.

## v0.6 Complete

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

## v0.7 Current Plan

- [x] Pin GitHub epic #213 for v0.7.
- [x] #214: Implement GitHub-native WorkOrder/Issue/PR/CI status sync.
- [x] #215: Implement host-managed worker action layer.
- [x] #216: Implement multi-repo source graph and remote snapshots.
- [x] #217: Implement release readiness and project status rollup.
- [ ] #218: Implement CoreRoom rename and compatibility migration. (in progress)

## Notes

- v0.7 work must keep #213 updated. An issue is not done until tracker checkbox
  and Evidence Ledger row are updated.
- Commands are automation, CI, debug, and recovery surface. The happy path is
  user intent -> `@host` -> role/gate/evidence/tracker orchestration.
- CoreRoom rename implementation is tracked in #218 and must preserve the `cr`
  happy-path command unless explicitly changed.
