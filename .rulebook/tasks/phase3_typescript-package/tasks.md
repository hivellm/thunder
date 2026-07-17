## 1. Implementation
- [ ] 1.1 Package skeleton: `@hivehub/thunder`, sole runtime dep `@msgpack/msgpack` ^3 (WIRE-031 - NOT msgpackr), tsup ESM+CJS dual build, Node >= 18 engines; tsc + eslint + vitest wiring (PKG-002, PKG-010)
- [ ] 1.2 Value: discriminated union `{kind, value}` over the 8 variants + factories/accessors per T-014 ergonomics; Int = bigint with number accepted on input for safe ranges; Bytes = Uint8Array
- [ ] 1.3 Wire codec: array-encoded Request/Response (WIRE-012), externally-tagged value forms incl. the `{"Ok":{"Str":…}}` nesting (WIRE-003), Bytes as msgpack bin (WIRE-010), compact ints + f64 bit-pattern preservation (WIRE-014), PUSH_ID
- [ ] 1.4 Streaming FrameReader (the Vectorizer codec.ts pattern) with the cap validated against the length prefix before allocation (WIRE-020/021 - closes the T-004 TS gap), partial input + multiple buffered frames (WIRE-022), typed FrameTooLarge/decode errors (WIRE-023)
- [ ] 1.5 Legacy decode tolerance: int-array Bytes normalized on decode (WIRE-011) - emit paths are bin-only
- [ ] 1.6 Client lifecycle: one node:net connection, TCP_NODELAY, connect timeout default 10 s (CLT-001); 3 handshake styles HelloMandatory/AuthCommand/None per profile with typed auth failures (CLT-002/003); explicit idempotent close failing in-flight calls (CLT-004)
- [ ] 1.7 Multiplexing: monotonic u32 ids skipping PUSH_ID, demux Map by id resolved from a background reader (CLT-010), serialized writes (CLT-011), max_in_flight backpressure (CLT-012), unknown-id drop (CLT-013), poison-on-malformed-frame (CLT-014)
- [ ] 1.8 Timeouts + cancellation: per-call timeout default 30 s, configurable per client and per call (CLT-020); AbortSignal honored, removing the pending entry on cancel (CLT-021)
- [ ] 1.9 Lazy reconnect: 2 attempts with capped backoff, re-handshake per profile, no silent replay of in-flight calls (CLT-030/031)
- [ ] 1.10 Typed errors: class (auth/server/connection/timeout/frame-too-large/decode) + optional code from the `"[code] "` prefix, NOAUTH/WRONGPASS → auth in both conventions, classes stable public API (CLT-050..052, WIRE-040)
- [ ] 1.11 Push hook: id == PUSH_ID routed to the registered handler, never matched against pending calls; protocol error under Reserved profiles (CLT-060)
- [ ] 1.12 Endpoint parser: `scheme://host[:port]` per profile registry + bare `host:port`; `http(s)://` rejected with a pointer to the product's HTTP client (CLT-070/071)
- [ ] 1.13 Corpus loader (~50 LOC) walking conformance/vectors/ and asserting per mode, in the DEFAULT vitest run - never feature-gated, never skipped (TST-020)

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation
- [ ] 2.2 Write tests covering the new behavior
- [ ] 2.3 Run tests and confirm they pass
