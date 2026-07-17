# SPEC-003 — Client Contract

| | |
|---|---|
| **Status** | Draft — public API shape freezes at G1 |
| **Phase / tasks** | Phase 1 · T1.4 + Phase 3 · T3.1–T3.3 + Phase 4 · T4.1 ([DAG](../DAG.md)) |
| **PRD requirements** | FR-20..FR-30; NFR-07 |
| **Requirement prefix** | `CLT-` |
| **Source** | Union of the best in-family clients (analysis [§1.3 T-003](../analysis/01-current-state.md), [§2 T-013/T-014](../analysis/02-module-design.md)); Vectorizer Rust client as the demux reference |

Requirement IDs `CLT-xxx`. This contract holds **identically in all four languages** — it is the
"uniform floor" (PRD NFR-07). Language-idiomatic surface (async/await, Promises, Tasks,
CancellationToken) is expected; semantics below are not negotiable.

---

## 1. Connection lifecycle

- **CLT-001** [P0] A client owns one TCP connection (pooling is a layer above, CLT-080). Connect
  SHALL apply a configurable timeout, default **10 s**, and set TCP_NODELAY.
- **CLT-002** [P0] After connect, the client SHALL perform the profile's handshake (SPEC-002
  PRO-001) before user calls proceed: `HelloMandatory` sends the HELLO map (`version` default 1,
  `token` or `api_key`, `client_name`) as the first frame; `AuthCommand` sends `HELLO`
  (optional) and `AUTH` when credentials are configured; `None` sends nothing.
- **CLT-003** [P0] Handshake failures surface as a typed auth error (CLT-051), not a generic I/O
  error. Auth state is per-connection and sticky — no per-call credentials.
- **CLT-004** [P0] Close SHALL be explicit and idempotent; dropping/disposing the client closes
  the socket and fails all in-flight calls with a typed connection-closed error.

## 2. Multiplexing

- **CLT-010** [P0] The client SHALL assign monotonically increasing `u32` ids (skipping
  `PUSH_ID`), allow **concurrent in-flight calls**, and demultiplex responses by `id` via a
  background reader (task / event loop / Task — per language).
- **CLT-011** [P0] Writes SHALL be serialized (lock / queue) so frames never interleave; reads are
  the reader's alone.
- **CLT-012** [P0] In-flight calls SHALL be bounded by the profile's `max_in_flight`; excess calls
  wait (backpressure), they are not refused.
- **CLT-013** [P0] A response whose `id` matches no pending call SHALL be dropped (counted in
  client stats if any), never treated as fatal.
- **CLT-014** [P0] A malformed frame from the server SHALL poison the connection: fail all pending
  calls with a typed error and close (the family's TS pattern). The next call MAY reconnect
  (CLT-030).

## 3. Timeouts and cancellation

- **CLT-020** [P0] Per-call timeout, default **30 s**, configurable per client and per call. On
  timeout the pending entry is removed; a late response to that id falls under CLT-013.
- **CLT-021** [P0] Where the language has first-class cancellation (`CancellationToken`, `ctx`,
  `AbortSignal`), calls SHALL honor it, removing the pending entry on cancel.

## 4. Reconnection

- **CLT-030** [P0] Lazy reconnect: when a call finds the connection dead/absent, the client SHALL
  re-dial (and re-handshake per profile) up to **2 attempts** with capped backoff before failing
  the call. No background reconnect loops.
- **CLT-031** [P0] Reconnection MUST NOT silently replay in-flight calls — calls that were pending
  when the connection died fail with the typed connection error; retry policy belongs to the
  product SDK (idempotency is product knowledge).

## 5. Errors

- **CLT-050** [P0] `Result::Err(string)` SHALL be parsed per the profile's `error_codes`
  (PRO-014) into a typed error carrying: raw message, optional `code` (from `"[code] "`), and
  error class (auth / server / connection / timeout / frame-too-large / decode).
- **CLT-051** [P0] `NOAUTH`/`WRONGPASS`/`NOPERM` prefixes map to the auth class regardless of
  convention (both existing conventions use them for auth). `NOPERM` — Synap's admin-ACL refusal
  (`synap_rpc/server.rs:243-245`) — was unmodeled until the BN-023 errata; it is an
  authorization refusal, classed with authentication because a client's recourse is the same:
  present different credentials.
- **CLT-052** [P0] Error classes are stable public API — product SDKs and user code branch on
  class and `code`, never on message text.

## 6. Push frames

- **CLT-060** [P0] Frames with `id == PUSH_ID` SHALL be routed to a registered push handler and
  never matched against pending calls. Under `push = Reserved` profiles, receiving one is a
  protocol error (poison per CLT-014); under `Enabled`, the handler receives the decoded `Value`.
- **CLT-061** [P1] A dedicated subscription helper (dedicated connection, Synap-style
  `SUBSCRIBE` flow) MAY ship as a convenience over CLT-060; the hook is the contract.

## 7. Endpoints

- **CLT-070** [P0] The endpoint parser SHALL accept `scheme://host[:port]` for every registered
  profile scheme (PRO-012) plus bare `host:port` (RPC implied — the caller supplies the profile).
  `http(s)://` URLs are rejected with a pointer to the product's HTTP client — Thunder is
  RPC-only.
- **CLT-071** [P0] Scheme → default port resolution comes from the profile registry; products
  never fork the parser.

## 8. Pooling

- **CLT-080** [P1] An optional pool (fixed N connections, round-robin checkout, no
  bb8/deadpool-style lifecycle) MAY ship per language, mirroring the Vectorizer pattern. The
  single-connection client is the primary contract.
  **Swap note**: Vectorizer's SDKs ship a pool today — until CLT-080 lands, the swap keeps a thin
  product-side pool wrapper over Thunder clients (the existing ~150-LOC pattern), so the swap is
  never blocked on this requirement.

## 9. The behavioral floor suite

- **CLT-090** [P0] A shared behavioral suite (DAG T4.1) SHALL verify in every language: pipelined
  out-of-order completion; cap refusal on oversized inbound frame without allocation; connect and
  call timeouts firing; 2-attempt reconnect then typed failure; push routing per profile;
  error-class mapping for both conventions; unknown-id drop; poison-on-malformed-frame. Passing
  this suite in all four languages is gate **G4**.
