## 1. Implementation
- [x] 1.1 thunder-bench crate with one no-op dispatch backend: echo / static-reply / sink - zero storage, zero business logic, the transport is the only thing measured (BEN-001)
- [x] 1.2 Thunder RPC listener via thunder-server over the no-op backend (BEN-001)
- [x] 1.3 axum HTTP/1.1 + JSON listener over the same backend, same process/host/runtime/allocator (BEN-001, FR-70) — delivered hand-rolled (T1.6 scope decision: no heavy new deps for a no-op harness); an axum lane may swap in at T4.2 if the parity review demands (documented in `rust/thunder-bench/src/http.rs`)
- [x] 1.4 Driver harness with provable parity: continuous pipelining with no inter-batch gaps (the Synap -P 16 lesson), identical concurrency model and measurement points across protocols (BEN-003) — one caveat recorded in artifact honesty notes: at depth = burst the Thunder stamp includes in-client write-lock queueing; stamp point unifies at T4.2
- [x] 1.5 Run discipline: release builds only, warmup discarded, N repetitions with dispersion reported (BEN-011)
- [x] 1.6 Scenario matrix as data: 64 B echo, 4 KiB reply, 10k-item bulk, 768×f32 embedding, 1k pipelined requests, connection storm - crossed with pipeline depths {1, 16} and connections {1, 4, 16, 64} (BEN-010, FR-71) — all six rows and the full sweep declared as data in `src/scenarios.rs`; skeleton runs sweep connections {1, 4} and mark bulk-10k / embedding-768 pending; full sweep + those two scenarios run at T4.3
- [x] 1.7 Per-cell metrics: p50, p99, qps, bytes-on-wire per op (BEN-010, FR-71)
- [x] 1.8 Artifact writer: JSON + md summary with machine/environment header (CPU, OS, kernel, governor) (BEN-011, BEN-030) — kernel/governor fields present but say "unknown (platform probe lands at T4.3)"; captured today: OS/arch, logical CPUs, hostname, rustc, build profile, runtime timestamp
- [x] 1.9 Commit one example run under bench-out/ proving the artifact format end to end (BEN-030, FR-72) — `bench-out/skeleton-example.{json,md}` (release build, ops=256 warmup=64 reps=2)

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [x] 2.1 Update or create documentation covering the implementation — full rustdoc on every module (`cargo doc -p thunder-bench`); scope, parity model and T4.x deferrals documented in `lib.rs`, `http.rs`, `driver.rs` and in every artifact's honesty notes
- [x] 2.2 Write tests covering the new behavior — 33 unit tests (backend modes, HTTP listener + JSON mapping, p50/p99/qps stats, scenario parsing/matrix, artifact env/JSON/md/date) + 3 integration smoke tests (echo on both lanes end to end, pending rows, tiny connection storm)
- [x] 2.3 Run tests and confirm they pass — full gate green from `rust/`: `cargo fmt --all`, `cargo clippy --workspace --all-features -- -D warnings`, `cargo test --workspace --all-features` (121 passed, 0 failed)
