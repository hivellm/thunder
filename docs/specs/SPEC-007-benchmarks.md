# SPEC-007 — Benchmarks & the G5 Gate

| | |
|---|---|
| **Status** | Draft — scenario matrix freezes at G4 |
| **Phase / tasks** | Phase 1 · T1.6 + Phase 4 · T4.2–T4.4 ([DAG](../DAG.md)) |
| **PRD requirements** | FR-70..FR-73; NFR-05 |
| **Requirement prefix** | `BEN-` |
| **Source** | Owner mandate ("Thunder vs Bolt and RESP3 — and we must always win"); committed evidence `Synap/docs/benchmarks/redis-vs-synap.md`, `Nexus/bench-out/`; analysis [§6 T-025/T-026](../analysis/06-benchmark-mandate.md) |

Requirement IDs `BEN-xxx`. Two principles govern everything here: **isolate the transport** (the
engine must not be in the measurement) and **harness parity** (a shootout is only as valid as its
weakest client — the lesson committed in Synap's own artifact, where a blocking bench client
understated the native path at pipeline depth 16).

---

## 1. The shootout harness

- **BEN-001** [P0] `thunder-bench` (Rust, in `rust/`) hosts **one no-op dispatch backend**
  (echo / static-reply / sink — zero storage, zero business logic) behind four listeners on the
  same process/host/runtime/allocator:

  | Listener | Basis |
  |---|---|
  | Thunder RPC | `thunder::server`, no-op dispatch |
  | RESP3 | reuse a family RESP3 implementation over the same backend |
  | Bolt | minimal Bolt v5 subset — handshake, HELLO, RUN, PULL, single-record results (PackStream encode/decode for exactly the matrix's shapes) |
  | HTTP/1.1 + JSON | axum + serde_json over the same backend |

- **BEN-002** [P0] The Bolt listener implements only what the matrix needs and its scope is
  documented in the artifact (honesty note: it is a benchmark peer, not a Bolt server product).
- **BEN-003** [P0] One driver per protocol with **provable parity**: continuous pipelining (no
  inter-batch gaps), identical concurrency model and measurement points. The RESP3 driver SHALL be
  validated against `redis-benchmark` on the same listener before its numbers are trusted
  (calibration step recorded in the artifact).

## 2. The scenario matrix

- **BEN-010** [P0] Cells = scenario × pipeline depth {1, 16} × connections {1, 4, 16, 64};
  metrics per cell = p50, p99, qps, bytes-on-wire per op. Scenarios:

  | Scenario | Probe |
  |---|---|
  | Point echo, 64 B | per-frame overhead floor |
  | Medium reply, 4 KiB | typical document/search response |
  | Bulk, 10k items one request | the family's strongest mandated row |
  | Embedding, 768×f32 | `Bytes` raw vs base64-JSON vs PackStream list — payload claim, measured |
  | Pipelined 1k requests | pipelining on merit (Bolt and RESP3 both pipeline) |
  | Connection storm (connect+first-byte ×1k) | handshake cost across the four protocols |

- **BEN-011** [P0] Runs are pinned: release builds, warmup discarded, N repetitions with
  dispersion reported, machine/environment header (CPU, OS, kernel, governor) in every artifact.
- **BEN-012** [P1] In-family end-to-end confirmations ride the same release: Nexus RPC-vs-RESP3 on
  the real engine (both listeners exist in one binary — the run the Lexum study flagged as
  missing), and a Synap rerun with the parity client to retire the `-P 16` asterisk.

## 3. The G5 gate — always win

- **BEN-020** [P0] **Gate G5**: for every cell of BEN-010, Thunder RPC beats RESP3, Bolt and HTTP
  on p50, p99 **and** qps, and is ≤ on bytes-on-wire where payload encoding differs. Required
  margin ≥ **10%**; a cell within ±10% is a *tie* → investigated and re-run before the gate can
  pass; a genuine loss anywhere **blocks the release** and becomes an optimization task
  (PRD NFR-05).
- **BEN-021** [P0] The optimization playbook for losing cells, in order: write coalescing /
  vectored writes → frame batch flush at pipeline depth → allocation reuse in the codec →
  semaphore/in-flight tuning. (Each lever has in-family precedent; Synap's `BufWriter` alone was
  +23%.)
- **BEN-022** [P0] G5 is evaluated per release train that touches wire/client/server hot paths;
  results are compared release-over-release and a regression that flips a cell re-blocks.

## 4. Artifacts and claims discipline

- **BEN-030** [P0] Results are **committed** under `bench-out/` (JSON + md summary, env headers),
  the way `Nexus/bench-out/` does — never left in git-ignored build dirs (the family gap this
  fixes).
- **BEN-031** [P0] Public claims cite artifacts by path/commit. "Transport-isolated" and
  "end-to-end" are never mixed in one sentence. No quantitative claim ships before G5 —
  READMEs may state *mechanisms* freely, numbers only with artifacts.
- **BEN-040** [P1] Product-level RPC-vs-HTTP harness (FR-73): bulk ingest / small high-QPS
  call / pipelined polling scenarios, same handlers both transports, runnable by each family
  product on its real engine, committed per-product (`bench-out/`-equivalent). Seeded from
  Nexus's acceptance table; products calibrate their own floors from the first measured run.
