# Proposal: phase5_push-streaming-design

## Why
Push exists in the family exactly once — Synap's SUBSCRIBE over the reserved PUSH_ID — and the 1.0 profile model only distinguishes Reserved vs Enabled (PRO-001). Other products will want server-initiated frames (watch, progress, invalidation), and without a family-level design each would invent its own: the exact drift Thunder exists to end. P2, post-1.0.

## What Changes
Design only — no implementation. A proposal for family push/streaming semantics beyond Synap's SUBSCRIBE, wire-compatible via the reserved PUSH_ID (id = u32::MAX) so the wire version stays 1 (WIRE-004/WIRE-005). Frame semantics are coordinated with Synap — the only product shipping push — before anything is specified. Deliverables: a SPEC-001 §push amendment proposal, a PRO-001 `push` profile-field evolution (growing beyond Reserved | Enabled with backward-compatible defaults), and corpus vectors for the proposed frames. Explicitly out of scope: chunked streaming — v2 territory, stays deferred.

## Impact
- Governing spec: SPEC-001 §push (WIRE-005) - docs/specs/SPEC-001-wire-format.md; SPEC-002 (PRO-001 push field)
- PRD requirements: P2 (post-1.0 fast-follow)
- DAG: T5.2; depends on G5 (1.0.0 shipped)
- Affected code: none — deliverable is a spec proposal + corpus vectors, not implementation
- Breaking change: NO (wire-compatible via reserved PUSH_ID; no version bump per WIRE-004)
- User benefit: one family answer for push/streaming instead of per-product reinvention, adoptable without breaking any deployed client
