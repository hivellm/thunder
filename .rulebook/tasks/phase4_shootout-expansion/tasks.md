## 1. Implementation
Each protocol lane = minimal listener + parity driver over the shared no-op backend
(BEN-001/002), continuous-pipelining parity (BEN-003), validated against the real
implementation, wired into Lane::ALL + the matrix runner + Targets. Phased by
value/cost. Results stay diagnostic until a quiet host produces a citable artifact.

Phase 1 — ceiling & variant (low cost):
- [ ] 1.1 Memcached binary protocol lane — the leanest possible FIFO peer; defines the performance ceiling (how close Thunder gets to a trivial protocol)
- [ ] 1.2 RESP2 lane — RESP3 minus the v3-only types; near-free given the existing RESP3 lane

Phase 2 — serious binary DB wires:
- [ ] 1.3 PostgreSQL v3 wire lane — startup + simple/extended query + DataRow; a mature, heavily-optimized DB wire
- [ ] 1.4 MongoDB OP_MSG lane — header + BSON body; a natural codec comparison (BSON vs MessagePack)

Phase 3 — binary RPCs:
- [ ] 1.5 Apache Thrift (TCompactProtocol over framed transport) lane
- [ ] 1.6 MessagePack-RPC lane — the Thunder sibling (msgpack array [type,msgid,method,params]); isolates Thunder's design from its codec

Phase 4 — the expensive, highest-value target:
- [ ] 1.7 gRPC lane (HTTP/2 framing + HPACK + length-prefixed protobuf) — multiplexed like Thunder; PROVES the d1/c4 sync-tiny cost is the universal price of multiplexing, and isolates our implementation quality in pipeline
- [ ] 1.8 Cap'n Proto RPC lane — zero-copy; strong on large payloads

Phase 5 — messaging (assess req/resp fit before building):
- [ ] 1.9 NATS req/reply lane — the closest messaging model to request/response
- [ ] 1.10 MQTT lane — CONNECT/PUBLISH/PUBACK; pub/sub, comparison is partial
- [ ] 1.11 AMQP / Kafka — evaluate whether a request/response peer is meaningful at all (streaming/pub-sub models); document the decision, do not force a misleading comparison

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation — each lane's protocol scope + honesty note (peer, not product) in its module docs and the matrix artifact; a comparison analysis in docs/analysis/ once >=1 phase lands
- [ ] 2.2 Write tests covering the new behavior — per-lane encode/decode + round-trip tests, mirroring bolt.rs/resp3.rs coverage
- [ ] 2.3 Run tests and confirm they pass — full Rust gate green after each lane; the citable matrix artifact remains gated on a quiet host (shared with phase4_hotpath)
