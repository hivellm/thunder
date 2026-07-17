## 1. Implementation
- [ ] 1.1 thunder-bench crate with one no-op dispatch backend: echo / static-reply / sink - zero storage, zero business logic, the transport is the only thing measured (BEN-001)
- [ ] 1.2 Thunder RPC listener via thunder-server over the no-op backend (BEN-001)
- [ ] 1.3 axum HTTP/1.1 + JSON listener over the same backend, same process/host/runtime/allocator (BEN-001, FR-70)
- [ ] 1.4 Driver harness with provable parity: continuous pipelining with no inter-batch gaps (the Synap -P 16 lesson), identical concurrency model and measurement points across protocols (BEN-003)
- [ ] 1.5 Run discipline: release builds only, warmup discarded, N repetitions with dispersion reported (BEN-011)
- [ ] 1.6 Scenario matrix as data: 64 B echo, 4 KiB reply, 10k-item bulk, 768×f32 embedding, 1k pipelined requests, connection storm - crossed with pipeline depths {1, 16} and connections {1, 4, 16, 64} (BEN-010, FR-71)
- [ ] 1.7 Per-cell metrics: p50, p99, qps, bytes-on-wire per op (BEN-010, FR-71)
- [ ] 1.8 Artifact writer: JSON + md summary with machine/environment header (CPU, OS, kernel, governor) (BEN-011, BEN-030)
- [ ] 1.9 Commit one example run under bench-out/ proving the artifact format end to end (BEN-030, FR-72)

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation
- [ ] 2.2 Write tests covering the new behavior
- [ ] 2.3 Run tests and confirm they pass
