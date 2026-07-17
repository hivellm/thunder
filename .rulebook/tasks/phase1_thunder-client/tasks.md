## 1. Implementation
- [x] 1.1 Crate skeleton + connect: one TCP connection per client, configurable connect timeout default 10 s, TCP_NODELAY on (CLT-001)
- [x] 1.2 Multiplexing core per the Vectorizer demux reference (analysis T-028): monotonically increasing u32 ids skipping PUSH_ID, background reader task, oneshot demux by id, serialized writes so frames never interleave (CLT-010/011)
- [x] 1.3 In-flight bound from the profile's max_in_flight with backpressure - excess calls wait, never refused (CLT-012)
- [x] 1.4 Unknown-id responses dropped and counted, never fatal (CLT-013); malformed server frame poisons the connection - fail all pending calls typed and close (CLT-014)
- [x] 1.5 Handshake per profile before user calls: None sends nothing; AuthCommand sends optional HELLO + AUTH when credentials configured; HelloMandatory sends the HELLO map (version default 1, token/api_key, client_name) as the first frame; failures surface as typed auth errors, auth sticky per connection (CLT-002/003)
- [x] 1.6 Per-call timeout default 30 s, configurable per client and per call; pending entry removed on timeout so late responses fall under unknown-id drop (CLT-020)
- [x] 1.7 Lazy reconnect: re-dial + re-handshake up to 2 attempts with capped backoff when a call finds the connection dead; in-flight calls that died fail with the typed connection error, never silently replayed (CLT-030/031)
- [x] 1.8 Typed errors: parse Result::Err per profile error_codes - NOAUTH/WRONGPASS map to the auth class, "[code] " yields a structured code; error classes are stable public API (CLT-050..052)
- [x] 1.9 Push hook: id == PUSH_ID frames routed to the registered handler under push = Enabled, protocol-error poison under Reserved, never matched to pending calls (CLT-060)
- [x] 1.10 Endpoint parser from the profile registry: scheme://host[:port] for every registered scheme plus bare host:port; http(s):// rejected with a pointer to the product's HTTP client (CLT-070/071)
- [x] 1.11 Explicit idempotent close; drop closes the socket and fails in-flight calls with a typed connection-closed error (CLT-004)
- [ ] 1.12 Optional TLS behind a rustls feature flag (FR-29) — deferred: the T0 TLS decision (rustls vs platform-native, cert config surface) is still open; no rustls dependency until it lands
- [x] 1.13 Integration tests covering the behavioral floor under every handshake style: pipelined out-of-order completion, per-call timeout, 2-attempt reconnect then typed failure, poison-on-malformed/oversized, push routing (feeds the CLT-090 floor suite) — run against loopback thunder-wire responders; thunder-server (T1.5) lands in parallel and the shared suite is T4.1

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [x] 2.1 Update or create documentation covering the implementation
- [x] 2.2 Write tests covering the new behavior
- [x] 2.3 Run tests and confirm they pass
