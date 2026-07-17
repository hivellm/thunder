## 1. Implementation
- [x] 1.1 RESP3 listener over the shared no-op backend: reuse a family implementation (e.g. nexus-protocol resp3), same process/host/runtime/allocator as the Thunder and HTTP listeners (BEN-001)
- [x] 1.2 Minimal Bolt v5 listener: handshake, HELLO, RUN, PULL, single-record results; PackStream encode/decode for exactly the matrix's shapes; scope documented in the artifact — benchmark peer, not a Bolt server product (BEN-001/BEN-002)
- [x] 1.3 Parity drivers, one per protocol: continuous pipelining with no inter-batch gaps, identical concurrency model and measurement points across all four (BEN-003)
- [~] 1.4 Calibration: PARTIAL. Protocol correctness VALIDATED against real redis:7-alpine tooling (redis-cli PING/ECHO/STATIC/SINK; redis-benchmark ping_mbulk -P16, 200k requests; --pipe rejected exactly as documented). Throughput calibration still UNRUN: redis-benchmark can only reach the listener via Docker NAT (~0.4ms RTT), which dominates and cannot rule out the understating-driver failure BEN-003 targets. Needs a host-native redis-benchmark. Recorded in resp3.rs; reproducible via --serve-resp3
- [x] 1.5 Matrix runner: 6 scenarios (point echo 64 B, medium 4 KiB, bulk 10k items, embedding 768xf32, pipelined 1k, connection storm) x depth {1,16} x connections {1,4,16,64}; p50/p99/qps/bytes-on-wire per cell (BEN-010)
- [x] 1.6 Pinned runs: release builds, warmup discarded, N repetitions with dispersion reported, machine/environment header (CPU, OS, kernel, governor) in every artifact (BEN-011)
- [x] 1.7 Commit results under bench-out/ (JSON + md summary, env headers) — never left in git-ignored build dirs (BEN-030)
- [x] 1.8 Evaluated G5: **FAIL** — Thunder clears the >=10% bar in 3 of 11 cells; 3 outright losses (-15.1% vs http at depth 16; -28.4% vs resp3 and -50.1% vs bolt on pipelined-1k). Verdict + per-cell table: bench-out/T4.3-G5-VERDICT.md
- [x] 1.9 Genuine losses: filed **phase4_hotpath-optimization** (release-blocking). Evidence redirects the playbook: the wire is the leanest lane in every cell, so the cost is per-request execution — leading suspect is spawn-per-request (SRV-003), which is not in the BEN-021 list
- [ ] 1.10 DEFERRED until G5 is green: in-family end-to-end confirmations (BEN-012). Running them now would produce numbers no claim may cite (BEN-031), and they touch product repos (owner-manual)
- [x] 1.11 Claims discipline holds: no quantitative claim ships while G5 is red. The verdict doc cites its artifact and separates transport-isolated from end-to-end

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation
- [ ] 2.2 Write tests covering the new behavior
- [ ] 2.3 Run tests and confirm they pass
