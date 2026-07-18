# Proposal: phase4_shootout-expansion

## Why
The T4.2/T4.3 shootout compares Thunder against the **family** competitors it exists
to replace — RESP3 (Synap), Bolt (Nexus/Neo4j), HTTP baseline. The owner wants
**breadth**: compare Thunder against the wider space of binary wire/RPC protocols, both
for public positioning ("why not X?") and — crucially — to **isolate the cost of
multiplexing**. The d1/c4 analysis showed Thunder trails a FIFO-simple peer (RESP3) on
sync-tiny payloads precisely because its out-of-order demux (the machinery that wins
+362% pipelined) has a per-call cost RESP3-FIFO does not pay. Adding **gRPC** — which is
*also* multiplexed (HTTP/2 streams) — would prove that trade-off is universal, not a
Thunder defect: gRPC should pay the same sync-tiny cost. Adding **Memcached binary** (the
leanest possible FIFO) defines the performance ceiling.

## What Changes
Add peer lanes to `thunder-bench`, each a **minimal listener + parity driver over the
shared no-op backend** (BEN-001/002 — a benchmark peer, not a product), with continuous-
pipelining parity (BEN-003) and validation against the real implementation. Phased by
value/cost, lowest-cost-highest-value first. Each lane joins the matrix runner; results
stay diagnostic until a quiet host produces a citable artifact (BEN-031). Every lane is
documented in the artifact as a peer, never presented as a product.

Phasing (each protocol is roughly a Bolt-sized effort, ~500-1600 LOC):
- Phase 1 — ceiling and variant (low cost): Memcached binary (lean FIFO ceiling), RESP2.
- Phase 2 — serious binary DB wires: PostgreSQL v3, MongoDB OP_MSG (BSON vs MessagePack).
- Phase 3 — binary RPCs: Apache Thrift (TCompactProtocol), MessagePack-RPC (the sibling).
- Phase 4 — the expensive, highest-value target: gRPC (HTTP/2 + protobuf; multiplexed
  like Thunder, resolves the d1/c4 question), Cap'n Proto RPC.
- Phase 5 — messaging (assess req/resp fit): NATS (req/reply), MQTT; AMQP/Kafka are
  streaming/pub-sub, so evaluate whether a request/response peer is meaningful and
  document the decision rather than force a bad comparison.

## Impact
- Governing spec: SPEC-007 (BEN-001/002/003/010/030/031)
- PRD requirements: breadth beyond the family mandate; public positioning
- Affected code: rust/thunder-bench/ (new per-protocol modules, Lane enum, matrix
  runner, Targets); no product code, wire frozen
- Breaking change: NO (harness only)
- User benefit: Thunder positioned against the whole binary-protocol field, with the
  multiplexing trade-off proven universal (gRPC) and the performance ceiling shown
  (Memcached) — measured, transport-isolated, not asserted
