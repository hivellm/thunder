## 1. Implementation
- [ ] 1.1 Inventory Synap's shipped push semantics — SUBSCRIBE flow, frame shapes, delivery guarantees — as the compatibility baseline (CLT-060/CLT-061 precedent)
- [ ] 1.2 Collect family push/streaming needs beyond SUBSCRIBE (candidate use cases across Nexus/Vectorizer/Lexum: watch, progress, invalidation)
- [ ] 1.3 Coordinate frame semantics with Synap — the only product shipping push — before specifying; record the agreement
- [ ] 1.4 Draft the SPEC-001 §push amendment: push/streaming frame semantics over the reserved PUSH_ID (id = u32::MAX), no wire version bump (WIRE-004/WIRE-005)
- [ ] 1.5 Draft the PRO-001 `push` field evolution: how the profile dimension grows beyond Reserved | Enabled with backward-compatible defaults (existing profiles unchanged)
- [ ] 1.6 Author corpus vectors for the proposed push frames, loadable and marked proposal-stage per SPEC-005 conventions
- [ ] 1.7 State the non-goals explicitly in the proposal: no implementation in this task; chunked streaming deferred to v2

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation
- [ ] 2.2 Write tests covering the new behavior
- [ ] 2.3 Run tests and confirm they pass
