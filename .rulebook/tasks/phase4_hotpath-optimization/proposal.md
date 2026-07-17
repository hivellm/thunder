# Proposal: phase4_hotpath-optimization

## Why
**G5 is red.** The first full shootout matrix (T4.3, artifact `bench-out/t4.3-full-matrix.json`,
verdict `bench-out/T4.3-G5-VERDICT.md`) shows Thunder clearing the BEN-020 bar (beat RESP3, Bolt and
HTTP by ≥10%) in **3 of 11 cells**. Three cells are outright losses: −15.1% vs HTTP at depth 16,
−28.4% vs RESP3 on pipelined-1k/4conns, −50.1% vs Bolt on pipelined-1k/1conn.

SPEC-007 BEN-021 makes a genuine loss **release-blocking** and requires an optimization task; BEN-031
bars any quantitative performance claim while the gate is red. The owner mandate is "precisamos ganhar
sempre" — this task is how that becomes true rather than asserted.

**The wire is not the problem.** Thunder is the leanest lane on the wire in every cell (83 B/op vs
HTTP's 4,192 on medium-4KiB), and HTTP still out-throughputs it at depth 16. The pattern is stark:
Thunder wins **+117% / +110%** with no concurrency (depth 1, 1 connection) and collapses as depth and
connections rise. The cost is **per-request execution**, and it appears above the codec.

## What Changes
Diagnose first, then optimize — the BEN-021 playbook is a list of usual suspects, not a diagnosis, and
the leading suspect here is not on it.

**Two hypotheses, ordered by the evidence:**
1. **Spawn-per-request** (`thunder::server`, SRV-003): every request acquires a semaphore permit and
   `tokio::spawn`s a task. At pipelined-1k that is ~1000 task spawns in flight per connection, while
   the RESP3/Bolt/HTTP peers answer inline in their read loop. This is the prime suspect for the
   −50% pipelined collapse and is **not** in the BEN-021 list.
2. **The client's serialized write mutex** (CLT-011): `driver.rs` already documents that the Thunder
   stamp at burst depth includes in-client write-lock queueing. That was written off as a latency
   artifact — but qps is down too, so it is not only the stamp.

**A fairness confound must be quantified before either is called a transport verdict** (see the
verdict doc): Thunder's listener carries PUSH_ID checks, first-frame handshake gating, session state,
a semaphore, spawn-per-request and per-op metrics; the peer listeners are minimal read loops with none
of it. That is a confound *against* Thunder and is not what BEN-001 asked for ("isolate the
transport"). Real Redis and Neo4j carry their own per-command overhead these peers do not. Measure how
much of the gap is Thunder's server features vs its transport **before** optimizing blind.

Then apply, in evidence order: inline-dispatch (or batched spawn) on the hot path → write coalescing /
vectored writes → frame batch flush at pipeline depth → allocation reuse in the codec → semaphore /
in-flight tuning. Re-run the matrix after each change; a cell is fixed only when the artifact says so.

## Impact
- Governing spec: SPEC-007 (BEN-020/021/031), SPEC-004 (SRV-003 in-flight bound, SRV-006 hot path)
- PRD requirements: NFR-05; gate G5
- DAG: follows T4.3; blocks G5 and therefore every Phase 5 task
- Affected code: `rust/thunder/src/server/listener.rs` (dispatch/spawn path), `rust/thunder/src/client/conn.rs`
  (write serialization), possibly `wire/frame.rs` (allocation reuse); `rust/thunder-bench` (peer parity
  instrumentation for the confound)
- Breaking change: NO intended — the wire is frozen and configs are data. Any change to SRV-003's
  spawn model must keep per-connection failure isolation (SRV-004) and the in-flight bound (SRV-003);
  if it cannot, that is a spec decision, not a silent optimization.
- User benefit: "beats Bolt and RESP3 — measured, transport-isolated, artifacts committed" becomes a
  citable fact instead of a target. Today it is not true, and the gate is doing its job by saying so.
