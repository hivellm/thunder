# §7 — Which of the Three Implementations Is the Performance Baseline?

> **Question**: of the three existing implementations (Synap origin, Nexus canonical, Vectorizer
> port), which is the most performant — the one Thunder should use as its base?
>
> **Answer in one line**: **no single one wins whole — the fastest Thunder is a composite: Synap's
> server write path, Vectorizer's client architecture, anyone's codec (they are byte-identical),
> plus one fix none of the three has (`Bytes` as bin)**. Per component, however, there is a clear
> ranking, evidenced below from hot-path code reads and an empirical rmp-serde probe.
>
> Method: full reads of the three server hot paths (`synap-server/src/protocol/synap_rpc/server.rs`,
> `nexus-server/src/protocol/rpc/server.rs`, `vectorizer-server/src/protocol/rpc/server.rs`), the
> three codecs, and a compiled probe against rmp-serde 1.3 (scratchpad `rmpprobe`, output quoted in
> T-029). Committed benchmark artifacts cited where they exist. **Caveat honesty**: the three
> servers have never been raced head-to-head — that is exactly what the §6 shootout does; this
> section ranks *mechanisms*, each traceable to a line of code.

## 7.1 Server hot path — Synap is the most performance-engineered

### T-027 — Synap's listener carries three optimizations the other two lack, and it is the only implementation with measured transport throughput

The per-operation write path, side by side:

| Dimension (per response) | **Synap** | Nexus | Vectorizer |
|---|---|---|---|
| Write buffering | ✅ **`BufWriter` + drain-then-flush**: after writing one response, `try_recv()` drains every queued response before a single `flush()` — a pipelined burst coalesces into one syscall (`server.rs:110-161`) | ❌ raw write half — one `write_all` syscall per response (`server.rs:116-152`) | ❌ raw write half (`server.rs:98-121`) |
| Serializations per response | **1** — `encode_frame` once, `write_all(&frame)` reuses it for the metrics length (`server.rs:126-134`) | **2** — `encode_frame(&response)` for the out-bytes metric, then `write_response` **re-encodes** internally (`server.rs:120-123` + codec `write_frame:131-138`) | **1** — `write_response` only; no frame-size metrics at all |
| Request re-encode for metrics | 1 extra `rmp_serde::to_vec(&req)` (`server.rs:198`) | 1 extra (`encoded_request_size`, `server.rs:227-230`) | 0 |
| `TCP_NODELAY` (server side) | ✅ explicit, with the delayed-ACK rationale documented (`server.rs:100-102`) | ❌ not set | ❌ not set |
| Auth check cost per request | plain `bool` in the serialized read loop — zero synchronization (`server.rs:170-235`) | `AtomicBool` behind `Arc` — cheap (`server.rs:157-162`) | `parking_lot::RwLock` read per request (`server.rs:128`) |
| Reply-value copies | ✅ `Bytes(Arc<[u8]>)` — GET replies carry the store's buffer with **no copy** ("phase11 wire-value zero-copy", `types.rs:20-26`) | `Vec<u8>` — copy per reply | `Vec<u8>` — copy per reply |
| In-flight bound per connection | ❌ none (only a max-connections semaphore) | ✅ semaphore, 1024 default | ❌ none — unbounded spawn |
| Idle/slow-loris timeout | ✅ per-read timeout (`server.rs:177-187`) | ❌ | ❌ |
| Frame cap | 512 MiB fixed | ✅ configurable (64 MiB default) | 64 MiB fixed |

**Measured evidence** (the only committed transport-throughput numbers in the family, T-025):
Synap's own artifact — SynapRPC **166k rps GET at `-P 1`** (~3× its RESP3 and Redis 7 on the same
host) and **600k rps GET at `-P 16`**, where the artifact records that adding the `BufWriter`
alone was **+23%** (`Synap/docs/benchmarks/redis-vs-synap.md`). Nexus's committed bench is
codec-only ("low microseconds" encode+decode, `protocol_point_read.rs`) plus end-to-end wins vs
Neo4j — real, but not comparable per-op transport numbers. Vectorizer has no isolated transport
benchmark.

**Why Nexus's server is structurally the slowest per-op of the three** despite being the most
observable: on every request it pays (a) one *extra full serialization of the response* (metrics
+ re-encode), (b) one extra full serialization of the *request* (size metric), and (c) one
unbuffered syscall per response with Nagle enabled. None of these shows up in its codec bench —
they live in the listener. They are all fixable, which is precisely what SPEC-004 SRV-006..008
mandates for Thunder.

- **Impact**: `thunder-server`'s hot path should be **ported from Synap's listener**, then add
  Nexus's two operational wins (per-connection in-flight semaphore, configurable cap) and Nexus's
  metrics *computed without re-encoding* (the codec returns the frame length it already knows).
- **Confidence**: high on mechanisms (file:line above); medium on magnitude until the §6 shootout
  races them (the +23% BufWriter figure is the one committed data point).

## 7.2 Client (Rust) — Vectorizer is the only real multiplexer

### T-028 — Vectorizer's Rust client is the only one of the three that can pipeline at all

| Dimension | Synap client | Nexus client | **Vectorizer client** |
|---|---|---|---|
| Concurrency | ❌ mutex held across write+read — single-flight (`transport/mod.rs:92-135`) | ❌ mutex single-flight + id assert (`transport/rpc.rs:39,83-88`) | ✅ **background reader + `oneshot` demux map** — true pipelining (`rpc/client.rs:186-291`) |
| Practical ceiling | 1 in-flight per connection | 1 in-flight per connection | bounded by server/in-flight config |
| Extras | push support, 2-try reconnect | — | pool |

A single-flight client caps *any* server at `1 / RTT` per connection regardless of server quality —
on the family's own pipelined-workload rows this is the difference between the `-P 1` and `-P 16`
columns of Synap's artifact (166k → 600k rps). For Thunder this is already decided
(SPEC-003 CLT-010, base = Vectorizer per DAG T1.4); this finding is the performance justification.

- **Impact**: Nexus and Synap Rust SDK users get the single largest client-side speedup of the
  whole program for free at the P2 swap: pipelining where today there is none.
- **Confidence**: high.

## 7.3 Wire/codec — byte-identical, and none of the three is optimal

### T-029 — Empirical probe: all three Rust implementations emit `Bytes` as an int-array; the bin form Thunder canonicalizes is ~33% smaller on embeddings and is already decodable by every existing server

Compiled probe against rmp-serde 1.3 (the version all three crates pin), scratchpad `rmpprobe`:

```
enum Value { …, Bytes(Vec<u8>), … }                    // Nexus & Vectorizer shape
Bytes([1, 2, 3, 255])  →  81 a5 4279746573 94 01 02 03 cc ff    ← msgpack ARRAY (255 costs 2 bytes)

enum Value { …, Bytes(#[serde(with="serde_bytes")] …) } // what none of them does
Bytes([1, 2, 3, 255])  →  81 a5 4279746573 c4 04 01 02 03 ff    ← msgpack BIN

plain Vec<u8> decodes the bin-encoded payload: OK      ← rmp-serde is lenient both ways
serde_bytes decodes the seq-encoded payload:  OK
```

Three consequences:

1. **This corrects §1 T-005 (item 1)**: the drift is not "Synap seq vs Nexus/Vectorizer bin" — plain
   `Vec<u8>` under rmp-serde also emits the int-array form, so **all three Rust servers and crates
   emit seq today**. Synap's `arc_bytes` comment ("matches `Vec<u8>`'s wire encoding — a plain seq
   of u8", `types.rs:31-35`) is the accurate one; Vectorizer's TS-side comment claiming rmp-serde
   emits bin (`codec.ts:48-51`) is wrong about the Rust default — its TS/Go/C# SDKs emit bin and
   the servers accept it only thanks to rmp-serde's leniency, which the probe confirms both ways.
2. **Payload math**: a random byte costs 1.5 bytes expected in the array form (values ≥ 128 encode
   as 2 bytes — the `cc ff` above) vs 1.0 in bin. A 768-dim f32 embedding = 3,072 bytes: **≈ 4,608
   bytes as emitted by every current Rust server, 3,077 as bin — Thunder's WIRE-010 beats all
   three existing implementations by ~33% on the family's flagship payload**, and the probe proves
   the change is safe against every deployed rmp-serde server (bin decodes fine).
3. Everything else in the codecs is identical in structure and cost across the three (same
   `to_vec` + prefix copy on encode, same `vec![0u8; len]` + `read_exact` on decode, cap before
   allocation) — there is no codec-level winner to pick; the wins are all in the layers above.

- **Impact**: the "4× smaller embeddings vs base64 JSON" family claim survives (raw-vs-base64
  dominates), but no current implementation achieves the optimal binary payload; Thunder does, by
  spec, from day one.
- **Confidence**: high — compiled, executed, hex-verified.

## 7.4 Verdict

### T-030 — Ranking and the composite baseline

**If forced to name one**: **Synap** is the most performant implementation as it stands — it is the
only one engineered *and measured* on the transport hot path (write coalescing +23%, nodelay,
zero-copy reply values, zero-sync auth), and the only one with committed rps numbers. Nexus is the
most complete and observable but pays two avoidable serializations per op and unbuffered writes on
its listener; Vectorizer is the leanest server but unbounded and unbuffered — its crown is the
client.

**But Thunder should not "pick one"** — the DAG already composes, and this section fixes the one
misassignment: T1.5 (`thunder-server`) bases its hot path on **Synap's listener**, not Nexus's,
while keeping Nexus's semaphore + configurable cap + metrics (computed without re-encoding, per
SPEC-004 SRV-006..008). T1.4 (client) stays Vectorizer-based (T-028). T1.1 (wire) stays
Nexus-sourced for the spec-completeness of the port — the bytes are identical anyway (T-029) — with
the bin canonicalization that beats all three.

| Thunder component | Base | Performance additions over the base |
|---|---|---|
| `thunder-wire` | `nexus-protocol` (most complete port source) | `Bytes` = bin (−33% on embeddings vs every current implementation) |
| `thunder-client` | Vectorizer Rust client | timeouts, reconnect, push hook (parity features, no perf cost) |
| `thunder-server` | **Synap listener** (BufWriter drain-then-flush, nodelay, inline auth, idle timeout) | Nexus's in-flight semaphore + configurable cap; metrics without re-encode; `Arc`-friendly value type for zero-copy replies |

The §6 shootout (G5) then races the composite against RESP3/Bolt/HTTP — and its per-cell margins
double as the regression harness proving this composite actually beats each donor implementation.

- **Confidence**: high on the composition; the magnitudes become numbers at T4.3.
