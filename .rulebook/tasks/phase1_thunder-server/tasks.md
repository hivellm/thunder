## 1. Implementation
- [x] 1.1 spawn_listener(dispatch, profile, config) -> ListenerHandle: TcpListener accept loop, one task per connection, graceful shutdown drains connections on handle drop/stop (SRV-001)
- [x] 1.2 Per-connection shape: split socket, dedicated writer task owning the write half behind an mpsc channel (SRV-002); read loop decodes with the profile cap and spawns one dispatch task per request bounded by a per-connection Semaphore sized by max_in_flight - excess waits, never refused (SRV-003)
- [x] 1.3 Hot path from the SYNAP listener (analysis §7 T-027): BufWriter + drain-then-flush - after one response, drain every queued response via try_recv before a single flush so a pipelined burst coalesces into one syscall (+23% in-family evidence) (SRV-006); set_nodelay(true) on every accepted connection (SRV-008); configurable per-read idle timeout, 0 disables (SRV-009)
- [x] 1.4 Exactly ONE serialization per response: encode once, write and measure that buffer for out-bytes; request in-bytes from the decoder's frame_len - re-encoding for metrics is banned (the Nexus anti-pattern) (SRV-007)
- [x] 1.5 Isolation: EOF/decode error ends the read loop and closes only that connection, never the listener (SRV-004); unknown commands return the profile's error convention and leave the connection usable (SRV-005)
- [x] 1.6 Sessions: Arc state with lock-free atomic auth flag flipped by HELLO/AUTH, read without locks (SRV-010); handshake enforcement per profile - HelloMandatory rejects non-HELLO first frames, AuthCommand applies the PING/HELLO/AUTH/QUIT pre-auth allowlist answering NOAUTH, None skips gating (SRV-011)
- [x] 1.7 Dispatch trait per SRV-020: dispatch(session, command, args) / authenticate(credentials) / capabilities(principal) - credential validation stays product-side, Thunder owns only the state machine (SRV-012); command matching byte-exact pass-through (SRV-022)
- [x] 1.8 HELLO reply construction by Thunder from ServerInfo + profile + authenticate/capabilities hooks, covering Nexus {server, version, proto, id, authenticated} and Vectorizer {protocol_version, capabilities} shapes, pinned by the corpus handshake group (SRV-014)
- [x] 1.9 PUSH_ID client frames refused with a dedicated error; under push = Enabled a typed per-connection PushSender stays valid for the connection lifetime so subscription flows emit after the registering request completed (SRV-013)
- [ ] 1.10 Metrics: 7 atomic series (connections, commands_total, commands_error_total, command_duration_microseconds_total, frame_bytes_in_total, frame_bytes_out_total, slow_commands_total) recorded after successful socket write (SRV-030); rpc.conn/rpc.req tracing spans with WARN on slow commands, threshold configurable (SRV-031)
  - SRV-030 metrics DONE (atomic series + configurable slow threshold, recorded after write, snapshot() on the handle); SRV-031 tracing spans deliberately deferred — no tracing dependency allowed at T1.5, slow commands are counted (slow_commands_total), not logged
- [x] 1.11 Error-formatting helpers for both family conventions - "[code] message" and ERR-prefixes - so products never hand-roll them (SRV-021)
- [x] 1.12 Integration suite per SRV-050, un-gated: ping over real TCP; 5-way multiplex out-of-order on one connection; PUSH_ID refusal; unknown-command survival; auth gating (reject -> HELLO/AUTH -> accept) per profile; oversized frame rejected without allocation; malformed body closes only that connection
- [ ] 1.13 Optional TLS: tokio-rustls behind a tls feature, config-gated via tls.cert_path/tls.key_path, no STARTTLS (SRV-040)

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [x] 2.1 Update or create documentation covering the implementation
- [x] 2.2 Write tests covering the new behavior
- [x] 2.3 Run tests and confirm they pass
