# Proposal: phase1_bench-skeleton

## Why
Gate G5 mandates Thunder win every matrix cell against RESP3, Bolt and HTTP (NFR-05), and the benchmark program is the longest chain in the DAG — which is why the shootout skeleton starts in Phase 1, not Phase 4 (DAG §3). Synap's own committed artifact carries the lesson: a blocking bench client understated the native path at pipeline depth 16, so harness parity must be designed in from the first run, not retrofitted.

## What Changes
Create the thunder-bench crate hosting one no-op dispatch backend (echo / static-reply / sink — zero storage, zero business logic) behind a Thunder RPC listener (thunder-server) and an axum HTTP/1.1+JSON listener on the same process/host/runtime/allocator. Build the driver harness with continuous pipelining (no inter-batch gaps — the Synap -P 16 lesson), warmup discard, and N repetitions with dispersion reported. The scenario matrix lives as data: 64 B echo, 4 KiB reply, 10k bulk, 768×f32 embedding, 1k pipelined, connection storm — crossed with pipeline depths {1,16} and connections {1,4,16,64} — producing p50/p99/qps/bytes-on-wire per cell. An artifact writer emits JSON + md with a machine/environment header, and one example run is committed to prove the artifact format end to end. RESP3 and Bolt listeners slot into this harness in Phase 4 (T4.2).

## Impact
- Governing spec: SPEC-007 (BEN-001, BEN-003, BEN-010/011, BEN-030) - docs/specs/SPEC-007-benchmarks.md
- PRD requirements: FR-70..FR-72
- DAG: T1.6 (gate G1); depends on phase1_thunder-client (T1.4) + phase1_thunder-server (T1.5)
- Affected code: rust/thunder-bench (new), bench-out/ (committed example artifact)
- Breaking change: NO (new internal crate, never published)
- User benefit: the G5 evidence machine exists from Phase 1 — transport-isolated, parity-honest numbers with committed artifacts, so performance claims are backed the day the gate is evaluated
