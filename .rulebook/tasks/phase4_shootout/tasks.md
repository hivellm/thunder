## 1. Implementation
- [ ] 1.1 RESP3 listener over the shared no-op backend: reuse a family implementation (e.g. nexus-protocol resp3), same process/host/runtime/allocator as the Thunder and HTTP listeners (BEN-001)
- [ ] 1.2 Minimal Bolt v5 listener: handshake, HELLO, RUN, PULL, single-record results; PackStream encode/decode for exactly the matrix's shapes; scope documented in the artifact — benchmark peer, not a Bolt server product (BEN-001/BEN-002)
- [ ] 1.3 Parity drivers, one per protocol: continuous pipelining with no inter-batch gaps, identical concurrency model and measurement points across all four (BEN-003)
- [ ] 1.4 Calibration step: validate the RESP3 driver against redis-benchmark on the same listener before trusting its numbers; record the calibration in the artifact (BEN-003 — the Synap -P16 lesson)
- [ ] 1.5 Matrix runner: 6 scenarios (point echo 64 B, medium 4 KiB, bulk 10k items, embedding 768xf32, pipelined 1k, connection storm) x depth {1,16} x connections {1,4,16,64}; p50/p99/qps/bytes-on-wire per cell (BEN-010)
- [ ] 1.6 Pinned runs: release builds, warmup discarded, N repetitions with dispersion reported, machine/environment header (CPU, OS, kernel, governor) in every artifact (BEN-011)
- [ ] 1.7 Commit results under bench-out/ (JSON + md summary, env headers) — never left in git-ignored build dirs (BEN-030)
- [ ] 1.8 Evaluate G5: Thunder beats RESP3, Bolt and HTTP on p50, p99 AND qps in every cell, <= bytes-on-wire where encoding differs; margin >= 10%; a cell within +/-10% is a tie — investigated and re-run before the gate passes (BEN-020)
- [ ] 1.9 Genuine losses: file a release-blocking optimization task per losing cell and apply the BEN-021 playbook in order — write coalescing/vectored writes -> frame batch flush at pipeline depth -> allocation reuse in the codec -> semaphore/in-flight tuning; re-run until the cell wins
- [ ] 1.10 In-family end-to-end confirmations per BEN-012: Nexus RPC-vs-RESP3 on the real engine (both listeners in one binary), Synap rerun with the parity client to retire the -P16 asterisk
- [ ] 1.11 Claims discipline: every public number cites its artifact by path/commit; "transport-isolated" and "end-to-end" never mixed in one sentence (BEN-031)

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation
- [ ] 2.2 Write tests covering the new behavior
- [ ] 2.3 Run tests and confirm they pass
