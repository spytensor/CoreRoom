# v0.6 Engineering Control Room Bootstrap Tasks

## Goal

Bootstrap v0.6 as the Engineering Control Room turn: host-led AI-assisted
software engineering control with GitHub issue discipline, dependency context,
evidence packets, and mandatory tracker closure.

## Current Plan

- [x] Pin GitHub epic #202 for v0.6.
- [x] Create v0.6 tracker and implementation issues (#202-#212).
- [x] Create v0.7 backlog tracker and issues (#213-#218).
- [x] Update GitHub repository description and topics toward Engineering Control Room positioning.
- [ ] #203: Land A-016 naming/product-positioning amendment.
- [ ] #204: Land A-017 host authority and host-led engineering control protocol.
- [ ] #205: Add AGENTS.md as the external AI worker protocol.
- [ ] #206: Implement host intent classification.
- [ ] #207: Define WorkOrder model and GitHub binding.
- [ ] #208: Define project Source Registry.
- [ ] #209: Define WorkOrder-scoped ContextPack.
- [ ] #210: Define Evidence Packet model.
- [ ] #211: Enforce tracker update protocol and PR evidence template.
- [ ] #212: Add end-to-end host-led dogfood validation.

## Notes

- v0.6 work must keep #202 updated. An issue is not done until tracker checkbox
  and Evidence Ledger row are updated.
- Do not pick up v0.7 issues while v0.6 is active unless the user explicitly
  re-scopes them into v0.6.
- Commands are automation, CI, debug, and recovery surface. The happy path is
  user intent -> `@host` -> role/gate/evidence/tracker orchestration.
- Repository/package/binary rename implementation is not part of #203. It is
  deferred to #218 unless the user explicitly changes scope.
