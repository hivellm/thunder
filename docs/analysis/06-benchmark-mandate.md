# §6 — Benchmark Mandate: Thunder vs Bolt vs RESP3 vs HTTP, and the Always-Win Gate

> Direction from the project owner: the family's protocol has already been raced against RESP3 and Bolt at the product level — Synap beats Redis in most cases and Nexus beats Neo4j in ~99% of cases — but for **Thunder** the confirmation must be 100%: a comparative benchmark of the Thunder protocol against Bolt and RESP3, and **Thunder must always win**. This section turns that mandate into a reproducible, defensible harness and a release gate.

## 6.1 What is already proven — the committed evidence, upgraded

### T-025 — The family already holds a committed, transport-isolated RPC-vs-RESP3 result (in Synap), and committed end-to-end wins over Bolt (in Nexus)

**Synap vs RESP3 vs Redis** (`Synap/docs/benchmarks/redis-vs-synap.md`, executed 2026-07-10, containers on one Docker network, same host, same server binary — the transport is the only variable between the first two columns):

| Op (`-n 100000 -c 50`, `-P 1`) | SynapRPC | RESP3 (same Synap server) | Redis 7.4.8 |
|---|---:|---:|---:|
| GET | **166,003 rps** | 56,116 | 56,022 |
| SET | **170,307 rps** | 52,301 | 56,465 |
| INCR | **169,726 rps** | 53,850 | 55,897 |

Per-op, native SynapRPC is **~3× RESP3 and ~3× Redis** — "a single MessagePack frame per reply vs RESP3's multi-segment bulk encoding, and a tighter codec" (the doc's own mechanism statement). This **partially supersedes the Lexum study's F-017** ("no RPC-vs-RESP3 numbers exist anywhere in the Nexus repo"): true for Nexus, but the family does hold a same-server, same-host comparison — in Synap.

The same document also carries the cautionary tale the Thunder harness must absorb: at `-P 16` (pipelined) the RESP3 rows beat the SynapRPC rows for GET/SET **because the SynapRPC load generator was a blocking batch client** that left inter-batch gaps while `redis-benchmark` kept the pipe full — the row "understates the SynapRPC server ceiling", and a `BufWriter` on the server side alone was worth +23%. Lesson: **a protocol shootout is only as valid as its weakest client**; harness parity is a correctness requirement, not a nicety.

**Nexus vs Neo4j-over-Bolt** (`Nexus/bench-out/serial-74.json`, `concurrent-combined.json`; analysis in `Nexus/docs/performance/BENCHMARK_2026.md`): committed end-to-end wins — serial point read p50 250 µs vs 2,305 µs; 34,764 vs 12,133 qps at 64 workers — with engine and transport deliberately conflated (F-016) and a handful of 64-worker counter-cells (aggregation, traversal) where Neo4j leads. This supports "Nexus beats Neo4j in the overwhelming majority of cases" as an **end-to-end** statement; it is not yet a transport-isolated Bolt comparison, which is exactly what §6.2 builds.

- **Impact**: Thunder does not start from zero — the direction is established by committed artifacts. What is missing is (a) Bolt isolated from the Neo4j engine, (b) RESP3 rerun with a harness-parity client, (c) one matrix, one host, one methodology for all three competitors at once.
- **Confidence**: high (the artifacts exist and say what is quoted; the -P 16 caveat is documented in the artifact itself).

## 6.2 The Thunder transport shootout — design

**Principle: isolate the transport.** All competitors run against the **same no-op dispatch engine** (echo / static-reply / sink) on the same host, same tokio runtime, same allocator, so the measured difference is framing + serialization + protocol state machine — nothing else. This closes the F-016 objection ("RPC-vs-Bolt conflates engine and transport") by removing the engine entirely.

**Servers** (one listener each, shared backend):

| Listener | Implementation basis |
|---|---|
| Thunder RPC | `thunder-server` with no-op dispatch |
| RESP3 | the family already ships two RESP3 server implementations (`nexus-protocol/src/resp3/`, Synap's 94-command listener) — reuse one over the same no-op backend |
| Bolt | minimal Bolt v5 subset (handshake, HELLO, RUN, PULL, one-record results) over the same backend — enough surface for the scenario matrix, small enough to keep honest (~ hundreds of LOC; PackStream is a msgpack-class binary encoding) |
| HTTP/1.1 + JSON | axum + serde_json over the same backend (keeps the family's headline HTTP claim in the same matrix) |

**Clients**: one driver per protocol with **provable harness parity** — continuous pipelining (no inter-batch gaps), identical concurrency model, identical measurement points. For RESP3, validate the driver against `redis-benchmark` numbers before trusting it. This is the -P 16 lesson institutionalized.

**Scenario matrix** (each cell: p50, p99, qps, and bytes-on-wire, at pipeline depths 1 and 16, connections 1/4/16/64):

| Scenario | Why it's in the matrix |
|---|---|
| Point echo, 64 B payload | per-frame overhead floor — where framing dominates |
| Medium reply, 4 KiB | typical document/search response |
| Bulk, 10k items in one request | the family's strongest mandated row (≥3.5× vs HTTP) |
| Embedding vector, 768×f32 | `Bytes` raw vs base64-JSON vs Bolt PackStream list — the 4× payload claim, measured |
| Pipelined 1k requests | the category HTTP loses by construction; Bolt/RESP3 both pipeline — must win here on merit |
| Connection storm (connect+first-byte ×1k) | handshake cost: Thunder HELLO vs Bolt handshake+HELLO vs RESP3 HELLO vs TCP+TLS+HTTP |

**In-family end-to-end confirmations** (secondary, same release): Nexus RPC-vs-RESP3 on the real engine (both listeners already exist in one binary — the run F-017 said was missing), and a Synap rerun with the parity client to retire the -P 16 asterisk.

## 6.3 The always-win gate

### T-026 — Gate G5: no Thunder release claims victory until every cell wins, and a losing cell is a release-blocking defect

**Gate definition**: for every scenario × depth × concurrency cell of §6.2's matrix, Thunder RPC must beat RESP3, Bolt and HTTP on p50, p99 **and** qps (and be ≤ bytes-on-wire where payload encoding differs). Margin ≥ 10% or the cell is investigated as a tie; a genuine loss anywhere **blocks the claim and the release** and becomes an optimization task before anything ships marketing language.

**Why "always win" is a rational gate and not bravado**: the structural levers are all on Thunder's side of the table — one 4-byte prefix + one binary body per frame (vs RESP3's multi-segment bulk replies, Bolt's chunked message framing, HTTP's text headers); zero per-request handshake or auth (connection-sticky); raw `Bytes` payloads (vs base64 or PackStream lists); and full pipelining with out-of-order completion. Where a cell loses, in-family history says the cause is an implementation artifact, not the protocol — Synap's two documented examples: the blocking bench client (fixed by harness parity) and the missing write coalescing (`BufWriter`, +23%). The optimization playbook for losing cells, in order: write coalescing / vectored writes → frame batch flush at pipeline depth → allocation reuse in the codec → semaphore/in-flight tuning.

**Claims discipline** (extends T-017): artifacts committed under `bench-out/`-style directories with environment headers; every public number cites its artifact; "transport-isolated" and "end-to-end" are never mixed in one sentence. The family's current gap — targets quoted as results (F-014) — is precisely what G5 exists to prevent for Thunder.

- **Impact**: after G5, the sentence the family has always wanted to publish — "the HiveLLM protocol beats Bolt and RESP3, measured, transport-isolated, artifacts committed" — becomes citable fact instead of extrapolation. Synap's committed 3× over RESP3/Redis at depth 1 says the expectation is realistic; the matrix makes it universal or shows exactly where work remains.
- **Confidence**: high that the gate is buildable and the harness design is sound; medium-high that every cell wins on the first run (the pipelined-GET history says at least one cell will demand an optimization pass — which is what the gate is for).

## 6.4 Plan wiring

- **P1** (§4) gains the no-op-backend shootout skeleton (Thunder + HTTP listeners first).
- **P4** (§4) becomes the shootout phase: RESP3 + Bolt listeners, parity clients, full matrix, G5 evaluated. The product-level RPC-vs-HTTP harness of §4 P4 remains (products confirm on their real engines), but the transport shootout is the primary artifact.
- **G5** is added after G4: **G4** = products meet their own RPC-vs-HTTP floors; **G5** = Thunder wins the transport matrix outright. Quantitative public claims unlock at G5, not before.
