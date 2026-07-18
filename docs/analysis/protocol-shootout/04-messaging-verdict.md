# §4 — Messaging systems: what was built, and what was refused

> Task `phase4_shootout-expansion` items 1.9, 1.10, 1.11. Governing spec:
> SPEC-007 (BEN-001, BEN-002).

Phase 5 of the expansion asked a question rather than mandating lanes:
**is a request/response peer meaningful for a messaging system at all, or
would building one force a misleading comparison?** The answer differs by
system, so this file records the decision for each and the reasoning behind
it.

## 4.1 The structural fact that governs all four

Every messaging system in scope puts a **broker between the endpoints**. That
is not an implementation detail — it doubles the network path:

```
  RPC lanes (11 of them):   client ──▶ server ──▶ client          2 traversals
  messaging lanes:          requester ──▶ broker ──▶ responder
                            requester ◀── broker ◀── responder    4 traversals
```

A messaging lane therefore **cannot be compared against a transport lane on
latency**. Doing so would report a topology difference as a protocol
difference, and the resulting number ("NATS is 4.7x slower than Thunder")
would be true, meaningless, and reliably misquoted.

What messaging lanes *can* be compared against is **each other**: same
topology, same traversal count, different wire. That comparison is
well-posed, and it is the one §3 draws between NATS and MQTT.

This is why the two lanes that were built are marked in the code as a
*different shape*, not merely a different protocol, and why neither is in
`Lane::ALL`.

## 4.2 NATS — built (item 1.9)

**Verdict: build.** NATS is the messaging system whose request/reply is
closest to a first-class pattern, and its cost is worth knowing precisely
because teams do reach for it as an RPC substitute.

- **Protocol**: line-oriented text, seven verbs, CRLF-framed. Small enough to
  implement faithfully.
- **Implementation split**: broker ours (no Rust NATS server exists — the
  reference implementation is Go and the ecosystem is client-only), client
  the real `async-nats` 0.49 on both requester and responder. A production
  client completing round trips against our broker is the validation that the
  wire is correct.
- **Faithfulness choice**: the responder is a *separate connection*, not
  folded into the broker. Collapsing it would have halved the traversals and
  measured nothing real.

## 4.3 MQTT — built (item 1.10)

**Verdict: build**, and specifically **MQTT 5**, because 3.1.1 has no
request/response affordance at all — you would have to invent a reply-topic
convention inside the payload, which measures your convention rather than the
protocol. MQTT 5 has `Response Topic` (0x08) and `Correlation Data` (0x09) as
real properties.

Its value is not as a Thunder competitor. It is as the **control for the NATS
lane**: identical broker topology, identical traversal count, a compact binary
wire against NATS's text one. That isolates the wire with the architecture
held constant.

- **QoS 0**, deliberately. QoS 1 doubles the packet count with PUBACKs, drags
  the broker's inflight-window bookkeeping into the measurement, and its
  window silently caps concurrency — a 500-concurrency run against a
  100-message window stalls on acks and reads as latency.
- **Implementation split**: both sides ours, a documented exception to the
  real-crates policy. `rumqttd` blocks in `start()` and spawns one OS thread
  per component, each with its *own* current-thread runtime — several extra
  runtimes outside the harness's control. Every codec crate is disqualified on
  maturity (`mqttbytes` dead since 2021, `mqtt-bytes-v5` effectively unused,
  `mqtt-protocol`'s v5 incomplete, `mqtt5-protocol` promising but young).
  MQTT is also the lane where owning the encoder costs least: fixed header
  plus a variable byte integer, small enough to verify exhaustively against
  the spec, which the tests do byte by byte.
- **The bias this creates is stated, not buried**: MQTT is hand-rolled on both
  sides while NATS runs a production client. §2.4 of this analysis documents
  that hand-rolled peers ran **2–4x faster** than production implementations
  in this very task. So the MQTT-vs-NATS gap is an **upper bound** on MQTT's
  advantage, not a measurement of it.

## 4.4 AMQP — refused (item 1.11)

**Verdict: do not build.**

AMQP's request/response is the RabbitMQ RPC convention: publish with a
`reply-to` queue and a `correlation-id`, consume the reply from that queue.
Structurally this is **the same four-traversal shape the NATS and MQTT lanes
already measure**. Building it would add a third data point to a question
already answered twice.

What AMQP genuinely adds over NATS/MQTT is *durability, acknowledgement
semantics, and exchange-based routing* — none of which are transport
properties, and all of which are deliberately absent from the no-op backend
the shootout isolates on (BEN-001). A lane that switched them off would not be
AMQP; a lane that left them on would be measuring queue durability, not a
wire.

Compounding it: no Rust AMQP **broker** exists (`lapin` is a client), so the
lane would be hand-rolled on both sides — the configuration §2.4 shows
flatters the protocol under test — to produce a number that duplicates an
existing one.

## 4.5 Kafka — refused, and the most strongly (item 1.11)

**Verdict: do not build.** This is the clearest refusal of the five.

1. **Kafka is not a request/response system.** It is an append-only
   partitioned log with poll-based consumers. "Request/response over Kafka"
   means writing to a request topic and polling a reply topic — a pattern
   Kafka supports the way a filesystem supports messaging.
2. **It persists to disk by design.** A Kafka lane would put fsync and page
   cache behaviour inside the measurement. That does not bend BEN-001
   ("isolate the transport, the engine must never be in the measurement") —
   it breaks it outright. Disabling durability to avoid this would produce a
   configuration nobody runs.
3. **It is optimized for the opposite axis.** Kafka's design centre is
   sustained throughput on large batches; its latency profile is dominated by
   `linger.ms` batching and replication acks. Measuring it on `point-echo-64B`
   would report the worst case of a system engineered for a different problem,
   and the number would be quoted as though it said something about Kafka.

A benchmark that measures a system on the axis it was explicitly not designed
for is not breadth. It is a category error with a chart attached.

## 4.6 Summary

| System | Built? | Reason |
|---|---|---|
| NATS | **yes** (1.9) | req/reply closest to first-class; real client validates our broker |
| MQTT 5 | **yes** (1.10) | control for NATS — same topology, different wire; Response Topic + Correlation Data are real protocol features |
| AMQP | **no** (1.11) | same 4-traversal shape already measured twice; its distinctive features (durability, acks, exchange routing) are not transport properties |
| Kafka | **no** (1.11) | not a request/response system; disk persistence breaks BEN-001; optimized for the opposite axis |

The refusals are the point of item 1.11, not a shortfall against it. The task
asked for a decision and its reasoning rather than two more lanes, precisely
so the artifact would not carry comparisons that look rigorous and mislead.
