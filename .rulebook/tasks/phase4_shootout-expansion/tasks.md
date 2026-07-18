## 1. Implementation
Each protocol lane = minimal listener + parity driver over the shared no-op backend
(BEN-001/002), continuous-pipelining parity (BEN-003), validated against the real
implementation, wired into Lane::ALL + the matrix runner + Targets. Phased by
value/cost. Results stay diagnostic until a quiet host produces a citable artifact.

Phase 1 — ceiling & variant (low cost):
- [x] 1.1 Memcached binary protocol lane — the leanest possible FIFO peer; defines the performance ceiling (how close Thunder gets to a trivial protocol)
      — `thunder-bench/src/memcached.rs`: minimal binary listener (one opcode, GET; key carries the workload via STATIC/SINK/PING sentinels, else echoed) + FIFO parity driver (sender+receiver via join!, VecDeque correlation, direct nodelay writes — the lean FIFO shape) + storm. Wired as a **reference** lane (Lane::Memcached in ALL_WITH_DIAGNOSTIC, NOT in Lane::ALL — a ceiling is not a peer Thunder must beat). 7 unit tests (echo/static/ping/sink round-trip, key-as-value, 250-byte cap rejection) green; validated end-to-end over sockets (bytes exact: 64B echo -> in 88/out 92; 4KiB -> in 30/out 4124). Full Rust gate green. Preliminary (allow-noisy, not citable): Thunder is not behind the FIFO ceiling — it wins several cells, e.g. point-echo d1/c1 31.6k vs 18.9k.
- [x] 1.2 RESP2 lane — RESP3 minus the v3-only types; near-free given the existing RESP3 lane
      — **RESOLVED BY ANALYSIS, not a duplicate lane.** For every measured scenario the existing RESP3 lane emits only RESP2-compatible types: ECHO/STATIC → `$` bulk string, bare PING → `+` simple string (resp3.rs `value_to_reply`/`process`). The v2/v3 difference is null encoding (RESP3 `_\r\n` vs RESP2 `$-1\r\n`), rich types (`,` `#` `(` `=`) and push (`>`) — none of which the echo/static/ping/pipelined cells exercise (no SINK/Null scenario is active). So a RESP2 lane would measure **byte-identical** traffic to RESP3; building it would present two "competitors" that are the same wire, which is misleading, not breadth. The RESP3 lane already IS the RESP2 comparison for these workloads. (If a SINK/Null scenario is ever added, revisit — that one cell would differ by the null encoding only.)

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
