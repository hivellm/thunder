## 1. Implementation
- [ ] 1.1 **Quantify the fairness confound first** (BEN-001): Thunder's listener does PUSH_ID checks, first-frame gating, session state, semaphore, spawn-per-request and per-op metrics; the peers do none of it. Measure the per-request cost of those features (e.g. a stripped Thunder listener as a 5th lane, or feature-by-feature ablation) so the gap is attributed to transport vs server features before anything is "optimized"
- [ ] 1.2 Profile the pipelined-1k cell (the −50% one) — do not guess. Confirm or kill hypothesis 1 (spawn-per-request: ~1000 `tokio::spawn` in flight per connection behind the SRV-003 semaphore) with real evidence
- [ ] 1.3 Confirm or kill hypothesis 2: the client's serialized write mutex (CLT-011). `driver.rs` documents it as a latency-stamp artifact, but qps is down too — establish whether it costs throughput
- [ ] 1.4 Fix in evidence order (BEN-021, amended by 1.2/1.3): inline dispatch or batched spawn on the hot path → write coalescing / vectored writes → frame batch flush at pipeline depth → allocation reuse in the codec → semaphore / in-flight tuning. One change at a time; re-run the matrix after each
- [ ] 1.5 Preserve the invariants any fix must not break: per-connection failure isolation (SRV-004), the in-flight bound (SRV-003), one serialization per response (SRV-007), and non-interleaved frames (CLT-011). If a fix requires relaxing one, that is a spec decision — raise it, do not do it silently
- [ ] 1.6 Re-run the full matrix per BEN-011 (release, warmup discarded, N reps, dispersion) and commit the artifact; a cell counts as fixed only when the artifact says so
- [ ] 1.7 Evaluate G5 again (BEN-020: ≥10% over every peer in every cell). If cells remain red, keep them red and say so — a losing cell blocks the release and bars every quantitative claim (BEN-031)
- [ ] 1.8 Related, non-blocking for this task but tracked: the RESP3 lane's throughput calibration is still unrun (Docker NAT dominated the reference run — see `resp3.rs`). It does not undermine these losses (it would only make RESP3 faster), but the lane's qps stays unverified until `redis-benchmark` runs on the host itself

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation — the verdict doc updated with the new matrix; SPEC-004 amended if the spawn model changes
- [ ] 2.2 Write tests covering the new behavior — the server suite must still prove SRV-003/004/007 and CLT-011 hold after the hot-path change
- [ ] 2.3 Run tests and confirm they pass — full gate green in all four languages, plus the committed matrix artifact showing the cells that moved
