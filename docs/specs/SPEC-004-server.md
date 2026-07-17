# SPEC-004 — Server (Rust)

| | |
|---|---|
| **Status** | Draft — dispatch trait freezes at G1 |
| **Phase / tasks** | Phase 1 · T1.5 + Phase 2 · T2.1–T2.3/T2.5 ([DAG](../DAG.md)) |
| **PRD requirements** | FR-40..FR-45 |
| **Requirement prefix** | `SRV-` |
| **Source** | The pattern all three family servers share (F-010); **hot path based on the Synap listener** per the §7 baseline analysis ([T-027/T-030](../analysis/07-performance-baseline.md)), operational features from Nexus (`nexus-server/src/protocol/rpc/server.rs`); analysis [§2 T-009](../analysis/02-module-design.md) |

Requirement IDs `SRV-xxx`. Rust only — every family server is Rust (analysis T-009). Products
integrate by implementing one trait; everything else (lifecycle, safety, metrics, handshake
enforcement) is Thunder's.

---

## 1. Connection lifecycle

- **SRV-001** [P0] `spawn_listener(dispatch, profile, config) -> ListenerHandle`: a
  `TcpListener` accept loop spawning one task per connection; graceful shutdown drains
  connections on handle drop/stop.
- **SRV-002** [P0] Per connection: the socket is split; a dedicated **writer task** owns the write
  half behind an mpsc channel so concurrent handlers complete out of order while frames serialize
  correctly on the wire (the family's proven shape).
- **SRV-003** [P0] The read loop decodes frames with the profile's cap (WIRE-020) and spawns one
  dispatch task per request, bounded by a per-connection `Semaphore` sized by the profile's
  `max_in_flight`. Excess requests wait; they are not refused.
- **SRV-004** [P0] EOF or decode error ends the read loop; dropping the channel drains the writer.
  A malformed frame closes that connection only — never the listener.
- **SRV-005** [P0] Unknown commands return the profile's error convention and MUST leave the
  connection usable (corpus-adjacent integration test, inherited from the Nexus suite).

## 1b. Hot-path requirements (from the §7 baseline analysis, T-027)

- **SRV-006** [P0] The writer SHALL buffer the write half (`BufWriter`) and use the Synap
  **drain-then-flush** pattern: after writing one response, drain every already-queued response
  (`try_recv`) before a single `flush()` — a pipelined burst coalesces into one syscall
  (committed in-family evidence: +23% from the buffer alone).
- **SRV-007** [P0] Exactly **one serialization per response**: the frame is encoded once and that
  buffer is written and measured (its length is the out-bytes metric). Request in-bytes come from
  the decoder's length prefix — the codec's read path SHALL return the frame size alongside the
  decoded value. Re-encoding a request or response for metrics is forbidden (the Nexus
  listener's double/triple-serialization is the anti-pattern this bans).
- **SRV-008** [P0] `set_nodelay(true)` on every accepted connection (delayed-ACK/Nagle interaction
  documented in the Synap listener).
- **SRV-009** [P0] Configurable per-read idle timeout (slow-loris resistance; `0` disables,
  matching each product's current posture via profile/config).

## 2. Sessions, auth, profile enforcement

- **SRV-010** [P0] Session state (auth flag, principal, capabilities) is an `Arc` with a lock-free
  atomic auth flag, flipped by `HELLO`/`AUTH` and read by the dispatch path without locks.
- **SRV-011** [P0] Handshake enforcement per SPEC-002 PRO-030: `HelloMandatory` rejects non-HELLO
  first frames; `AuthCommand` applies the `PING/HELLO/AUTH/QUIT` pre-auth allowlist and answers
  `NOAUTH …` otherwise; `None` skips gating.
- **SRV-012** [P0] Credential **validation is product code**: the dispatch trait receives the
  HELLO/AUTH payload via a `authenticate(credentials) -> Result<Principal, AuthError>` hook;
  Thunder owns the state machine, never the credential store.
- **SRV-013** [P0] Client frames with `PUSH_ID` are refused with a dedicated error (WIRE-005);
  under `push = Enabled` profiles the dispatch layer MAY emit push frames through a typed
  `PushSender` handed to it. The `PushSender` is per-connection and remains valid for the
  connection's lifetime, so subscription-style flows (Synap `SUBSCRIBE`) can emit long after the
  registering request completed.
- **SRV-014** [P0] HELLO replies are constructed by Thunder, not by product code, from three
  inputs: a `ServerInfo { name, version }` given at `spawn_listener`, the profile
  (`proto`/`protocol_version`), and the `authenticate`/`capabilities` hooks (`authenticated`,
  `capabilities`, connection id). This covers both family reply shapes — Nexus
  `{server, version, proto, id, authenticated}` and Vectorizer
  `{protocol_version, capabilities}` — pinned by the corpus handshake group (TST-015).

## 3. The dispatch trait

- **SRV-020** [P0] Product integration is exactly:

  ```rust
  #[async_trait]
  pub trait Dispatch: Send + Sync + 'static {
      async fn dispatch(&self, session: &Session, command: &str, args: Vec<Value>)
          -> Result<Value, String>;
      async fn authenticate(&self, creds: Credentials) -> Result<Principal, AuthError>;
      fn capabilities(&self, principal: &Principal) -> Vec<String> { vec![] }
  }
  ```

  Command routing, arg extraction and business logic are product-side (the Nexus modular
  `dispatch/` layout is the recommended structure, not part of this contract).
- **SRV-021** [P0] The error `String` returned by `dispatch` travels verbatim (WIRE-040);
  formatting helpers for the two family conventions (`"[code] message"`, `ERR`-prefixes) SHALL be
  provided so products don't hand-roll them.
- **SRV-022** [P0] Command name matching is byte-exact pass-through; case policy (Nexus uppercases,
  Vectorizer exact-match) is product-side inside `dispatch`.

## 4. Observability

- **SRV-030** [P0] Metrics as plain atomics, snapshot-friendly for any exporter:
  `connections` (gauge), `commands_total`, `commands_error_total`,
  `command_duration_microseconds_total`, `frame_bytes_in_total`, `frame_bytes_out_total`,
  `slow_commands_total` (threshold configurable). Metrics record **after** a successful socket
  write, per the Nexus writer contract.
- **SRV-031** [P0] Tracing spans `rpc.conn` / `rpc.req` with a WARN on slow commands
  (configurable threshold), matching the family's operating posture.

## 5. TLS

- **SRV-040** [P1] Optional `tokio-rustls`, config-gated (`tls.cert_path` / `tls.key_path`),
  feature-gated in the crate (`tls`). No STARTTLS. Default bind guidance stays loopback/private —
  the family's documented posture.
  **Ordering constraint**: Vectorizer already ships config-gated rustls on its RPC listener; if
  any Vectorizer deployment has it enabled, this requirement is pulled forward into M1 so the
  T2.2 swap is not a regression — verify at T0 and record the decision.

## 6. Safety tests (inherited matrix)

- **SRV-050** [P0] The server integration suite SHALL include, un-gated: ping round-trip over real
  TCP; 5-way multiplexing on one connection with out-of-order completion; PUSH_ID refusal;
  unknown-command survival; auth gating (reject → HELLO/AUTH → accept) per profile; oversized
  frame rejected without allocation; malformed body closes only that connection.
