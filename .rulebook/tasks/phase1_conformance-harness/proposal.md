# Proposal: phase1_conformance-harness

## Why
Byte-compatibility across four languages and three server products is currently a convention; corpus v0 (T0.4) covers only PING/PONG and basic framing. The family's documented anti-pattern is golden tests that exist but are feature-gated or ignored, and a wire library pinned only to itself proves nothing — Thunder must be pinned to the family's shipping reference (nexus-protocol) and exercised in the default test run.

## What Changes
Extend the corpus to the full 1.0 floor: value matrix (every variant alone and nested, empty containers, i64 extremes and compact-int boundaries, NaN bit pattern/±∞/−0.0, Err plain + "[code] " prefix + NOAUTH/WRONGPASS), completed framing edges with no-allocation reject vectors, tolerance group as decode-only (int-array Bytes, map-shaped Request), push vector with id = u32::MAX, and the handshake group (Nexus HELLO positional, Vectorizer HELLO map + capabilities reply). Ship the Rust corpus loader in the DEFAULT test run (no feature gates — NFR-03), cross-decode against nexus-protocol as a dev-dependency in both directions, and a deterministic pairwise-fuzz seed generator whose auto-shrunk failures graduate into permanent corpus vectors. CI gets a corpus lane per PR and a nightly rolling-seed fuzz lane, plus a coverage check that every SPEC-001 MUST maps to at least one vector.

## Impact
- Governing spec: SPEC-005 (TST-010..016, TST-020/021, TST-030, TST-040/041) - docs/specs/SPEC-005-conformance.md
- PRD requirements: FR-50..FR-53; NFR-03
- DAG: T1.2 + T1.3 (gate G1); depends on phase1_thunder-wire (T1.1)
- Affected code: conformance/vectors (extended), rust/thunder-wire test suite (loader, cross-decode, fuzz generator), CI workflows
- Breaking change: NO (corpus format is additive per TST-003; existing canonical bytes never change)
- User benefit: wire compatibility becomes an enforced CI property instead of a convention — every later port and product swap is checked against the same frozen vectors
