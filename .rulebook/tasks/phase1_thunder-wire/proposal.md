# Proposal: phase1_thunder-wire

## Why
The wire codec exists in 18 independently maintained copies (analysis T-001). thunder-wire is the single implementation everything else consumes, and it fixes the one wire-level suboptimality none of the three donors fixed: Bytes as int-array (~50% bigger than bin on embeddings, probe T-029).

## What Changes
Port the wire layer from nexus-protocol/src/rpc/ (most complete source; bytes identical across the family): 8-variant Value, Request/Response (array-encoded), PUSH_ID, frame codec with configurable cap checked before allocation. Additions over the donor: Bytes emitted as MessagePack bin via serde_bytes (WIRE-010), decode tolerances for legacy int-array Bytes and map-shaped Request (WIRE-011/013), and a read API that returns the frame size alongside the value so metrics never re-encode (feeds SRV-007). Pure crate: no tokio in core; async helpers behind a tokio feature.

## Impact
- Governing spec: SPEC-001 (WIRE-001..040) - docs/specs/SPEC-001-wire-format.md
- PRD requirements: FR-01..FR-06; NFR-01, NFR-02, NFR-09
- DAG: T1.1 (gate G1); depends on G0
- Affected code: rust/thunder-wire (new)
- Breaking change: NO (new crate; canonical bin is decode-compatible with every deployed server, probe-verified)
- User benefit: one codec, 33% smaller embedding payloads, cap-before-allocation everywhere it is consumed
