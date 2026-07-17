# Proposal: phase4_shootout

## Why
The owner mandate is explicit: Thunder vs RESP3 and Bolt, and Thunder must always win. The committed family evidence points the right way (Synap ~3x RESP3/Redis at depth 1; Nexus over Neo4j end-to-end) but no result is transport-isolated across all competitors on one host with one methodology — and Synap's own artifact documents the -P16 failure where a blocking bench client understated the native path (analysis §6 T-025). This task turns "always win" into the checkable gate G5.

## What Changes
Complete the shootout on the phase-1 skeleton (T1.6): add a RESP3 listener (reuse a family implementation, e.g. nexus-protocol resp3) and a minimal Bolt v5 listener (handshake, HELLO, RUN, PULL; PackStream for exactly the matrix's shapes; scope documented in the artifact per BEN-002 — a benchmark peer, not a Bolt product) over the same no-op backend; parity drivers per protocol with continuous pipelining, the RESP3 driver validated against redis-benchmark before its numbers are trusted (BEN-003 — the Synap -P16 lesson institutionalized); the full matrix run (6 scenarios x depths 1/16 x connections 1/4/16/64, p50/p99/qps/bytes per cell) with pinned runs; artifacts committed under bench-out/ with env headers (BEN-030). Then G5 is evaluated: Thunder beats RESP3, Bolt and HTTP in every cell with margin >= 10%; ties are investigated and re-run; a genuine loss is release-blocking and becomes an optimization task following the BEN-021 playbook (write coalescing -> batch flush -> allocation reuse -> in-flight tuning). In-family end-to-end confirmations ride the same release per BEN-012: Nexus RPC-vs-RESP3 on the real engine, Synap rerun with the parity client to retire the -P16 asterisk.

## Impact
- Governing spec: SPEC-007 (BEN-001..031) - docs/specs/SPEC-007-benchmarks.md
- PRD requirements: FR-70..FR-72; NFR-05
- DAG: T4.2 + T4.3 (gate G5); depends on G3
- Affected code: rust/thunder-bench (RESP3 + Bolt listeners, parity drivers, matrix runner); bench-out/ (new committed artifacts)
- Breaking change: NO (harness only; losing cells spawn hot-path optimization tasks)
- User benefit: "beats Bolt and RESP3 — measured, transport-isolated, artifacts committed" becomes citable fact; quantitative public claims unlock at G5
