# §3 — Draft SPEC-001 §push amendment (tasks 1.4, 1.7)

**Proposed** new section for SPEC-001, requirement prefix `WIRE-`, numbered in the
push band `WIRE-05x` to sit beside the existing WIRE-005. This is a *draft for
coordination*, not an edit to the frozen spec — nothing here is normative until
[05](05-coordination-and-open-questions.md) closes and the vectors are promoted.

The governing constraint is unchanged and absolute: **the wire version stays `1`**
(WIRE-004), `PUSH_ID = u32::MAX` stays the reserved server-initiated id (WIRE-005).
Everything below lives *inside* the `Value` payload of a frame that already exists.

---

## Proposed §6 — Push envelope (WIRE-050..WIRE-056)

- **WIRE-050** [P1] **The push envelope is the canonical shape of a server-initiated
  frame's payload.** A push frame SHALL be `Response{id: PUSH_ID, result: Ok(env)}`
  where `env` is a `Map` (WIRE-002, ordered) carrying exactly:

  | Key (`Str`) | Value | Meaning |
  |---|---|---|
  | `"stream"` | `Int` (u32 range) | The logical stream this frame belongs to. Equals the `id` of the `Request` that opened the stream (WIRE-051). `0` is the reserved **broadcast** stream — an uncorrelated frame sent to every consumer (WIRE-052). |
  | `"kind"` | `Str` | The event kind (WIRE-053). |
  | `"data"` | any `Value` | The product payload; opaque to Thunder (WIRE-055). |

  Keys SHALL be emitted in the order `stream`, `kind`, `data`. Decoders SHALL accept
  any key order and SHALL treat an absent `data` as `Null`. This whole structure is an
  ordinary `Map` value — no new wire construct, no version bump (WIRE-004 holds).

- **WIRE-051** [P1] **Correlation reuses the request id space.** The `stream` value
  SHALL be the `id` the client used on the `Request` that opened the stream (the
  `WATCH`/`SUBSCRIBE`/job-start call). The client already allocates monotonically
  increasing ids skipping `PUSH_ID` (CLT-010), so no new namespace is introduced and a
  client can route a push frame to the same call site that opened it. A server MUST NOT
  use `PUSH_ID` itself as a `stream` value.

- **WIRE-052** [P1] **`stream = 0` is broadcast.** Since `0` is never a client request
  id under CLT-010's allocator (ids start at `1`), it is free to mean "not tied to any
  one request" — the cache-invalidation / fan-out lane (PUSH-012). A consumer receives
  broadcast frames whenever `push` is `Streaming`, with no prior correlated request.

- **WIRE-053** [P1] **`kind` is an open vocabulary with five reserved values.** Thunder
  reserves and defines the routing semantics of:

  | `kind` | Semantics (Thunder-level) |
  |---|---|
  | `"data"` | A stream item. The degenerate single-`data` stream is exactly today's `SUBSCRIBE` (PUSH-013). |
  | `"progress"` | A non-terminal incremental update for `stream`. `data` is product-shaped (e.g. `{done,total,stage}`). |
  | `"end"` | **Terminal.** No further frames for `stream` will arrive; the client MAY free per-stream state. `data` MAY carry a final result or `Null`. |
  | `"error"` | **Terminal, abnormal.** `stream` ended in failure; `data` carries a product error (string or structure). |
  | `"invalidate"` | Cache/state invalidation; typically on the broadcast stream (WIRE-052). `data` carries affected keys/reason. |

  Products MAY define additional `kind` strings; Thunder SHALL route by `stream` and
  surface `kind` verbatim without interpreting unknown kinds. Reserved kinds SHALL NOT
  be redefined by a product.

- **WIRE-054** [P1] **Terminal kinds close a stream.** After an `"end"` or `"error"`
  for a given `stream` (≠ 0), a server SHALL NOT emit further frames for that `stream`
  without a new opening request; a client MAY reject or ignore late frames for a closed
  stream. The broadcast stream `0` never terminates.

- **WIRE-055** [P1] **`data` is opaque (purity preserved).** Thunder SHALL NOT parse,
  validate, or depend on the contents of `data` in any language — the wire layer
  carries no product knowledge (WIRE-030). Only `stream` (routing) and `kind`
  (dispatch/terminality) are Thunder-level concerns.

- **WIRE-056** [P1] **The envelope is profile-gated, not wire-gated.** Whether a client
  interprets `Ok(Value)` as a bare value (legacy `SUBSCRIBE`) or as this envelope is
  set by the `push` profile field (SPEC-002 PRO-001, evolved in
  [04](04-pro-001-push-field-evolution.md)), never by sniffing the bytes. Under
  `push = Enabled` the payload is delivered verbatim (PUSH-013 unchanged); under
  `push = Streaming` it is parsed as this envelope. A `Streaming` client receiving a
  payload that is not a well-formed envelope SHALL treat it as a protocol error
  (CLT-014 poison), symmetric to how `Reserved` treats any push frame.

---

## Client-side contract sketch (informative — SPEC-003 CLT-06x forward-look)

Normative client rules would be drafted as CLT-062..CLT-064 when this is ratified; the
shape:

- The push hook under `Streaming` receives `(stream, kind, data)` rather than a bare
  `Value`; the bare-`Value` hook (CLT-060) remains the `Enabled` contract.
- A convenience subscription handle keyed by `stream` MAY be layered on top (the
  CLT-061 "dedicated subscription helper" generalized from one channel to many); the
  hook stays the contract.
- Terminal kinds (`end`/`error`) complete any per-stream handle exactly once.

## Server-side contract sketch (informative — SPEC-004 SRV forward-look)

Normative server rules would be SRV-011..SRV-012; the shape: a push-emit API that takes
`(stream, kind, data)` and serializes one enveloped frame through the existing writer
task (SRV-002/SRV-007 unchanged — still one serialization per frame, still the single
writer). No change to the read/dispatch/spawn model.

---

## Non-goals (task 1.7) — PUSH-040

Stated explicitly so scope creep is visible:

- **PUSH-040a** — **No implementation in this task.** Deliverable is this proposal plus
  proposal-stage corpus vectors. Client/server code lands in a later task once the
  envelope is ratified.
- **PUSH-040b** — **Chunked streaming is v2.** Fragmenting a single large `data` value
  across multiple frames (with reassembly and per-chunk framing) is out. The envelope
  models many logical *streams*, not fragmentation of one *value*. A large payload is
  still one frame bounded by `max_frame_bytes` (WIRE-020).
- **PUSH-040c** — **No flow-control/credit protocol.** No window negotiation or
  backpressure signaling this round (PUSH-016). The product governs emission rate.
- **PUSH-040d** — **No new command.** `SUBSCRIBE`/`WATCH`/job-start stay product
  catalog commands. Thunder standardizes only the *envelope of the emitted frames*, not
  the commands that open streams (they remain per-product, like `CYPHER` or `SUBSCRIBE`
  are today).
- **PUSH-040e** — **`Enabled` is frozen.** The existing bare-`Value` behavior is not
  deprecated or altered by this proposal; `Streaming` is strictly additive.
