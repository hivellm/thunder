# Proposal: phase4_product-harness

## Why
The shootout isolates the transport; products still have to prove the win holds on their real engines. The family's standing gap is targets quoted as results — BEN-040/FR-73 close it by requiring a runnable RPC-vs-HTTP harness per product and a committed artifact behind every number. At least one committed product artifact is part of gate G4.

## What Changes
A product-level RPC-vs-HTTP harness that each family product runs on its own real engine: three scenarios — bulk ingest, small high-QPS call, pipelined polling — with the same handlers behind both transports so at product level the transport is the only variable. Acceptance floors are seeded from Nexus's table (point read 320µs -> ≤120µs, bulk 780ms -> ≤220ms baselines) and each product calibrates its own floors from its first measured run — seeds are not results. Results land in a bench-out/-equivalent directory with env headers, committed per product; no quantitative claim ships without its committed artifact.

## Impact
- Governing spec: SPEC-007 (BEN-040, claims per BEN-031) - docs/specs/SPEC-007-benchmarks.md
- PRD requirements: FR-73
- DAG: T4.4 (gate G4); depends on G3
- Affected code: harness template in rust/thunder-bench; per-product harness wiring + bench-out/-equivalent artifacts in Nexus, Vectorizer, Synap
- Breaking change: NO
- User benefit: each product states its own RPC-vs-HTTP numbers on its real engine, backed by a committed artifact instead of extrapolation
