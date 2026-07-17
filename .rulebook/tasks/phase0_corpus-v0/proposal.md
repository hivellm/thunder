# Proposal: phase0_corpus-v0

## Why
The corpus must exist before any implementation so all four language ports are written corpus-first (DAG T0.4, analysis T-015). The two family-pinned golden vectors plus the framing set are enough to gate G0.

## What Changes
Create `conformance/vectors/` with the TST-001 YAML schema and the v0 set: canonical PING request (`08 00 00 00 93 01 a4 'PING' 90`) and the nested `{"Ok":{"Str":"PONG"}}` response (from `VECTORIZER_RPC.md` §11 / vectorizer-protocol byte tests), plus the framing group (two frames in one buffer, partial header, partial body, zero-length body, cap-boundary, cap+1 reject-without-allocation). A small schema validator proves the files parse in CI.

## Impact
- Governing spec: SPEC-005 (TST-001..003, TST-010, TST-012)
- PRD requirements: FR-50
- DAG: T0.4 (gate G0)
- Affected code: conformance/vectors/ (new), CI validator lane
- Breaking change: NO
- User benefit: the byte-compatibility target exists before the first line of codec code
