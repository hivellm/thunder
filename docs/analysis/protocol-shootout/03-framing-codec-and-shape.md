# §3 — Isolating the variables: framing, codec, and shape

> Each comparison below holds everything constant but one thing. Numbers from
> the 14-lane matrix, `--allow-noisy`, 2000 ops/rep, 3 reps.

## 3.1 Framing, with the codec held constant — what the length prefix costs

**`msgpack-rpc` vs `thunder`.** The MessagePack-RPC lane encodes with *the same
`rmp-serde` version* the Thunder crate frames with. The only difference is
framing: MessagePack-RPC is a bare self-delimiting stream (a reader must
structurally scan the codec to find where a message ends), Thunder prefixes
every body with a `u32` length (WIRE-001).

| | thunder | msgpack-rpc |
|---|---|---|
| bytes in/out per op | 85 / 83 | **80 / 79** |
| point-echo d1/c1 | **27 370 qps** | 20 754 |
| point-echo d1/c4 | 52 476 | **55 589** |
| pipelined-1k d1000/c1 | **448 171** | 113 184 |

**The prefix costs 4–5 bytes per message and buys everything else.** Thunder
is 32% faster at d1/c1 and **4.0x faster** at depth 1000, with the codec
identical. MessagePack-RPC lands exactly where the FIFO peers land (113k,
against RESP3's 108k and Memcached's 103k) — which is the finding: with the
codec held constant, **Thunder's pipelined advantage is its design, not its
serialization**.

The 4-byte cost is also not quite 4 bytes net: Thunder spends the prefix but
saves a message-type element that MessagePack-RPC's `[type, msgid, method,
params]` array carries, so the wire difference is 5 bytes in and 4 out.

## 3.2 Codec, with the framing held constant — what encoding aggressiveness buys

**`thrift` vs `msgpack-rpc`.** Thrift's framed transport is a 4-byte
big-endian length prefix — Thunder's exact shape. So Thrift shares framing
with Thunder and differs on codec, the mirror image of §3.1. And
`TCompactProtocol` is the most aggressively size-optimized encoding in the
field: varint + zigzag integers, field ids written as *deltas*, booleans
folded into their field header.

| | msgpack-rpc | thrift |
|---|---|---|
| bytes in/out per op | 80 / 79 | 85 / 80 |
| point-echo d1/c1 | 20 754 qps | 20 701 |
| point-echo d16/c1 | 97 301 | 96 882 |
| pipelined-1k d1000/c1 | 113 184 | 110 229 |

**Statistically indistinguishable, and TCompactProtocol is not even smaller
here.** The reason is payload dominance: at a 64-byte payload, the encoding
governs only the handful of bytes around it. Compact's cleverness is spent on
integer fields and field ids, and this workload has almost none.

The honest conclusion is a *scope* conclusion, not a verdict on Thrift:
**codec choice is close to irrelevant for this shape of traffic.** Where
codecs would separate is many-small-fields structured records — which the
BEN-010 matrix does not contain, and which would need a new scenario to
measure. Choosing a wire format for codec efficiency on request/response
payloads of this shape is optimizing something that does not move.

## 3.3 The response *structure*, not the bytes — PostgreSQL

PostgreSQL v3 is typed and length-prefixed like Thunder, and its request is
the **leanest in the entire field** — 75 bytes on point-echo, against
Thunder's 85. Then it answers every query with four messages:
`RowDescription` + `DataRow` + `CommandComplete` + `ReadyForQuery`.

| | thunder | postgres (`pgwire`) |
|---|---|---|
| bytes in / out | 85 / 83 | **75** / 122 |
| point-echo d1/c1 | 27 370 qps | 11 797 |
| point-echo d16/c1 | 148 640 | 31 074 |
| pipelined-1k d1000/c1 | 448 171 | **19 325** |

Leanest request, fattest response, and last place under pipelining. Two
separate causes, and they must not be conflated:

1. **The protocol's own cost**: a four-message response cycle is structural
   per-response overhead no amount of implementation care removes.
2. **The implementation's cost**: `pgwire` *sends* (feed **and** flush) on
   `CommandComplete` and again on `send_ready_for_query` — roughly two flushes
   per query, where every other lane defers its flush while more input is
   buffered.

§1.3 quantifies the split: our hand-rolled peer, at **byte-identical**
traffic, ran 2–4x faster than `pgwire`. So the protocol permits roughly
25–100k qps here; the production Rust implementation delivers 19k. Reporting
either number as "PostgreSQL's speed" without the other is misleading, and
this analysis reports both.

## 3.4 Zero-copy did not pay — Cap'n Proto

Cap'n Proto is the one lane with **no parse step**: field access is pointer
arithmetic into the received buffer, where every other lane runs a decode pass
building a value.

| | thunder | capnp |
|---|---|---|
| bytes in / out | 85 / 83 | **248 / 165** |
| point-echo d1/c1 | 27 370 qps | 10 378 |
| pipelined-1k d1000/c1 | 448 171 | 56 433 |

**The heaviest wire in the field, by a wide margin** — three times Thunder's
request bytes. Cap'n Proto RPC level 1 spends `Call` + `Return` + `Finish`
messages per call, plus question/answer table bookkeeping and capability
descriptors. It removed the parse and added a protocol.

The lesson generalizes past this lane: **for small request/response payloads,
parse cost was never the bottleneck.** Eliminating it entirely bought nothing
that the added protocol overhead did not immediately spend. Zero-copy is a
real advantage for large structured messages read repeatedly — a shape this
matrix does not contain and Thunder's workloads do not have.

**Caveat**: this lane runs single-threaded on a dedicated current-thread
runtime because `RpcSystem` is `!Send` (§1.2). Its d1/c1 latency is
comparable; its aggregate throughput is not, and the pipelined figure above
should be read as "this lane on one thread", not as Cap'n Proto's ceiling.

## 3.5 Shape, not protocol — the broker tax

The NATS and MQTT lanes put a broker between the endpoints, so a round trip
crosses **four** sockets rather than two. They are not comparable with the
transport lanes (§4.1). They are comparable with **each other**: identical
topology, identical traversal count, different wire.

| | nats (text) | mqtt (binary) |
|---|---|---|
| bytes in / out | 270 / 274 | 204 / 204 |
| point-echo d1/c1 | 7 004 qps (140.0 µs) | **16 372** (49.3 µs) |
| point-echo d16/c1 | 49 999 | 46 760 |
| pipelined-1k d1000/c1 | 49 292 | 56 140 |

At depth 1 the binary wire is **2.8x faster** than the text one. But the gap
closes to nothing under concurrency (46.8k vs 50.0k at d16/c1) — at depth, both
are bounded by the broker's routing, not by parsing.

**This comparison carries the largest bias in the analysis and must be read
with it**: the MQTT lane is hand-rolled on both sides, while NATS runs the
real `async-nats` client. §1.3 showed hand-rolled peers running 2–4x faster
than production implementations. The 2.8x MQTT advantage at d1/c1 is therefore
an **upper bound**, and quite possibly most of it is the bias rather than the
wire.

The broker tax itself, against a point-to-point transport doing the same
logical work:

| | thunder (2 traversals) | nats (4) | mqtt (4) |
|---|---|---|---|
| point-echo d1/c1 | 35.9 µs | 140.0 µs | 49.3 µs |
| bytes in / out | 85 / 83 | 270 / 274 | 204 / 204 |

Roughly **2.4–3.9x the wire bytes** and, for NATS, **3.9x the latency**. That
is the cost of the architecture, and it buys things the RPC lanes do not
offer — decoupling, fan-out, subscription. Whether that is worth paying is a
design question this benchmark cannot answer; what it can do is price it
honestly.
