# Protocol Shootout — Thunder against the binary-protocol field

> **Status**: complete, **not yet citable**. Task
> `phase4_shootout-expansion`. Governing spec: SPEC-007
> (BEN-001/002/003/010/011/020/030/031).
>
> **Question**: the T4.2/T4.3 shootout compared Thunder against the *family*
> competitors it exists to replace (RESP3, Bolt, HTTP). This expansion asks
> the wider question — how does Thunder stand against the whole binary
> request/response field, and specifically: **is its sync-tiny deficit the
> universal price of multiplexing, or a Thunder defect?**
>
> **Answer**: the price of multiplexing, and Thunder pays it ~9x more cheaply
> than gRPC, the only other multiplexed peer. Meanwhile Thunder leads the
> entire field on d1/c1 latency — including the Memcached lane built to be the
> floor nobody beats — and runs 3.5x the best FIFO peer under pipelining.

## Ten new lanes, fourteen total

Every lane serves the **same** `NoopBackend` in the same process, host,
runtime and allocator (BEN-001), through a driver with one concurrency model
and one measurement point (BEN-003).

| Lane | Isolates | Implementation |
|---|---|---|
| `thunder` | the subject | — |
| `resp3` · `bolt` · `http` | the family peers (G5) | ours (pre-existing) |
| `thunder-stripped` | server features vs wire | ours (diagnostic) |
| `memcached` | the FIFO floor | ours |
| `mongodb` | BSON vs MessagePack | **real `bson` codec** |
| `postgres` | multi-message response cycle | **real `pgwire` server** |
| `msgpack-rpc` | framing (same codec, no prefix) | **real `rmp-serde`** |
| `thrift` | codec (same framing, TCompact) | **real `TCompactProtocol`** |
| `grpc` | **multiplexing** | **real `tonic`, both sides** |
| `capnp` | no parse step | **real `capnp-rpc`, both sides** |
| `nats` | broker topology, text wire | our broker, **real `async-nats`** |
| `mqtt` | broker topology, binary wire | ours |

AMQP and Kafka were **evaluated and refused**, with reasoning — see §4.

## Section index

| § | File | Contents |
|---|---|---|
| §1 | [01-method-and-lanes.md](01-method-and-lanes.md) | What each lane isolates; the mid-task switch to real protocol crates; **the finding that hand-rolled peers run 2–4x faster than production ones**; where parity is knowingly broken |
| §2 | [02-the-multiplexing-question.md](02-the-multiplexing-question.md) | The question the expansion existed for, answered against gRPC — the only other multiplexed peer |
| §3 | [03-framing-codec-and-shape.md](03-framing-codec-and-shape.md) | One variable at a time: what the length prefix costs, whether codec aggressiveness matters, response structure, zero-copy, the broker tax |
| §4 | [04-messaging-verdict.md](04-messaging-verdict.md) | NATS and MQTT built; **AMQP and Kafka refused**, and why refusing was the deliverable |
| §5 | [05-conclusions.md](05-conclusions.md) | What was established, what was not, what to do next |

## The five results worth remembering

1. **Thunder beats the floor.** It is faster at d1/c1 than the Memcached lane
   built to be the unbeatable baseline — 35.9 µs vs 42.5 µs. (§5.1)
2. **The multiplexing cost is real, universal, and cheap for Thunder.** Both
   multiplexed protocols trail FIFO at d1/c4: Thunder by 6%, gRPC by 57%. In
   every concurrent cell the trade pays back 49–253%. (§2)
3. **The length prefix is worth 4–5 bytes.** Against the same codec with no
   prefix, Thunder is 4.0x faster at depth 1000. (§3.1)
4. **Codec choice and zero-copy did not move this workload.** TCompactProtocol
   is statistically identical to MessagePack here; Cap'n Proto removed the
   parse step entirely and ended up with the heaviest wire in the field. Two
   negative results, and the more actionable ones. (§3.2, §3.4)
5. **Hand-written benchmark peers flatter protocols, they do not sabotage
   them.** Swapping our PostgreSQL peer for real `pgwire` at byte-identical
   traffic cost 2–4x throughput. (§1.3, §5.4)

## Reading discipline

**These numbers are not citable.** They were produced with `--allow-noisy` on
a developer workstation; BEN-031 requires a quiet host and that gate is still
open. What survives noise are large repeatable ratios (3.5x, 8.2x, 2–4x), and
this analysis claims only those — see [§5.5](05-conclusions.md) for the full
list of what these numbers are *not*, including which lanes are full-stack
rather than wire-vs-wire comparisons.

Every lane is a **benchmark peer, not a product** (BEN-002), documents its own
protocol scope in its module docs, and none of the ten new lanes is in
`Lane::ALL` — no "Thunder beats X" claim rests on any of them.
