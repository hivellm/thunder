# §1 — Method: what each lane isolates, and how honest each one is

> Task `phase4_shootout-expansion`. Governing spec: SPEC-007
> (BEN-001, BEN-002, BEN-003, BEN-011).

## 1.1 The two rules everything obeys

**BEN-001 — isolate the transport.** Every lane serves the *same*
`NoopBackend` (echo / 4 KiB static / sink / ping — zero storage, zero business
logic) in the same process, on the same host, runtime and allocator. Nothing
but the wire differs. Where a lane could not honour this, it says so in its
module docs and this file records it below.

**BEN-003 — harness parity.** One driver shape, one measurement point: a
continuously-full window of `depth` outstanding requests per connection,
latency stamped from just-before-send to reply-fully-consumed, warmup
discarded, N repetitions with dispersion reported (BEN-011).

## 1.2 The mid-task policy change that reshaped the results

The expansion began with hand-written peers. Partway through, the owner
directed a switch: **use real protocol crates wherever a Rust implementation
exists**, on the grounds that a peer written by the benchmark author is the
worst bias in a shootout.

A survey established what was actually available, and the answer was less than
hoped:

| Protocol | Real Rust **server**? | What we used |
|---|---|---|
| PostgreSQL | yes — `pgwire` 0.40 | real server, our driver |
| gRPC | yes — `tonic` 0.14 | **real on both sides** |
| Cap'n Proto | yes — `capnp-rpc` 0.26 | **real on both sides** |
| Thrift | server exists but is **blocking** | real codec, our I/O |
| MongoDB | **none** (official crate is a client) | real `bson` codec, our OP_MSG framing |
| MessagePack-RPC | **none usable** — `rmp-rpc` pulls tokio **0.1.22** | real `rmp-serde` codec (same one Thunder uses), our framing |
| NATS | **none** (reference server is Go) | our broker, real `async-nats` client |
| Memcached | **none** | ours |
| MQTT | `rumqttd` spawns its own runtimes | ours (see §4.3) |

Two rejections are worth stating because they were close calls:

- **`rmp-rpc`** would have given a real MessagePack-RPC server, but it depends
  on **tokio 0.1.22** — a different major version with an incompatible
  reactor, unmaintained since 2019, and with no declared licence. Running it
  would have put two reactors of different vintages in one process. The lane
  would have measured *tokio 0.1 vs tokio 1.x*, not MessagePack-RPC vs
  Thunder.
- **`rumqttd`** blocks in `start()` and internally builds one current-thread
  runtime per component thread — several extra schedulers outside the
  harness's control.

**The one exception granted**: `capnp-rpc` is `!Send` and requires a dedicated
current-thread runtime, which is also a second runtime instance. It was
allowed where `rmp-rpc` was not, and the distinction is real: `capnp-rpc` uses
the *same tokio 1.x, same allocator, same process*, differing only in
scheduler instance, while `rmp-rpc` meant a different tokio era entirely. The
cost is that the Cap'n Proto lane is single-threaded and **its aggregate
throughput is not comparable** — only its depth=1/conns=1 latency is. That is
reported rather than engineered around.

## 1.3 The finding that justified the whole policy change

When the PostgreSQL lane was swapped from our hand-written v3 listener to the
real `pgwire`, **the wire bytes were byte-for-byte identical** (75 in / 122 out
on point-echo; 12 in / 4154 out on medium-4KiB). Only the implementation
changed. The result:

| cell | our hand-rolled peer | production `pgwire` |
|---|---|---|
| point-echo d1/c1 | 20.8k qps | **11.0k** |
| point-echo d16/c1 | 93.7k qps | **19.6k** |
| pipelined-1k c1 | 105k qps | **25.2k** |

The hand-rolled peer was **2–4x faster than the production implementation**,
because it implemented the optimal drain-then-flush that `pgwire` does not
(`pgwire` sends — feed *and* flush — on `CommandComplete` and again on
`ReadyForQuery`, roughly two flushes per query).

The direction of this bias is the important part, and it is the opposite of
the intuition:

> **A benchmark peer written by the benchmark author does not sabotage the
> competitor. It flatters it.**

Anywhere a lane is still hand-rolled, its numbers should be read as an
**upper bound on what that protocol can do**, not as what a real
implementation of it delivers. Both readings are legitimate and they answer
different questions — but they must not be mixed in one table.

## 1.4 Lane inventory: what each one isolates

Fourteen lanes. Four are the original G5 peers (`Lane::ALL`); the rest are
reference or diagnostic and are deliberately excluded from any "Thunder beats
X" claim.

### The family peers (G5, in `Lane::ALL`)

| Lane | Isolates |
|---|---|
| `thunder` | the subject |
| `resp3` | the Redis/Synap convention — FIFO, text-ish, minimal |
| `bolt` | the Neo4j/Nexus competitor — PackStream, FIFO |
| `http` | HTTP/1.1 + JSON, the universal baseline |

### Reference lanes (breadth, `--diagnostic` only)

| Lane | Isolates | Implementation |
|---|---|---|
| `thunder-stripped` | what Thunder's *server features* cost vs its wire | ours (diagnostic) |
| `memcached` | the **FIFO performance ceiling** — 24-byte fixed header, one opcode | ours |
| `mongodb` | **BSON vs MessagePack**, codec held against a length-prefixed FIFO wire | real `bson` codec |
| `postgres` | the **multi-message response cycle** (4 messages per query) | real `pgwire` |
| `msgpack-rpc` | **framing**: same codec as Thunder, *no length prefix* | real `rmp-serde` |
| `thrift` | **encoding**: same framing shape as Thunder (4-byte prefix), different codec | real `TCompactProtocol` |
| `grpc` | **multiplexing** — the only other out-of-order peer | real `tonic`, both sides |
| `capnp` | **no parse step** — field access is pointer arithmetic | real `capnp-rpc`, both sides |
| `nats` | the **broker topology** (4 traversals), text wire | our broker, real client |
| `mqtt` | the same topology with a **binary** wire — the control for NATS | ours |

### The deliberate pairing

Two lanes were built to isolate the two halves of "what is a wire?" against
Thunder:

- **`msgpack-rpc`** — *same codec* as Thunder (literally the same `rmp-serde`
  version), differing only in framing (self-delimiting stream vs `u32` length
  prefix).
- **`thrift`** — *same framing shape* as Thunder (4-byte length prefix over a
  framed transport), differing only in codec (TCompactProtocol vs
  MessagePack).

Between them, both variables are isolated one at a time. Neither comparison
would be interpretable alone.

## 1.5 Where parity is knowingly broken, and why

Three lanes diverge from the standard driver. All three are documented in
their modules; none is silent.

| Lane | Divergence | Why it is correct here |
|---|---|---|
| `grpc` | driver is tonic's real client, not ours | HTTP/2 + HPACK cannot be hand-rolled without measuring the hand-roll; and "depth" on a multiplexed wire means N *concurrent streams*, not N writes before N reads — forcing the FIFO shape would serialize the property under test |
| `capnp` | client and server on a dedicated current-thread runtime | `RpcSystem` is `!Send`; no alternative exists |
| `nats`/`mqtt` | four socket traversals, not two | that *is* the architecture being priced (§4) |

For the gRPC lane specifically, one property is **asserted rather than
assumed**: the listener counts accepted connections and a cell fails outright
if a hidden pool opened more than the one intended per driver connection.
Without that check, every per-connection number would have been unverifiable.

## 1.6 What is still pending

The matrix in this analysis was produced with `--allow-noisy` on a developer
workstation. **No number here is citable at G5.** BEN-031 requires a quiet
host before the artifact is quotable; that gate is shared with
`phase4_hotpath` and remains open. The findings that survive noise are
*orderings and ratios that are large and repeatable*, and §5 is careful to
claim only those.
