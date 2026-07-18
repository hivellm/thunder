# §1 — The compatibility baseline: Synap `SUBSCRIBE` (task 1.1)

Synap is the family's **only** shipped push producer. Any family push design is an
extension of what `SUBSCRIBE` already puts on the wire, and must keep it working
unchanged. This section inventories exactly what that is, from what the Thunder repo
already documents about Synap's implementation (analysis §1, behavioral-normalization
§3) and Thunder's own frozen contract (SPEC-001 WIRE-005, SPEC-003 CLT-060, SPEC-004
SRV-002).

## The shipped flow

- **PUSH-001** — **Command path.** `SUBSCRIBE` is an ordinary command dispatched on
  the RPC path (`synap/.../dispatch/advanced.rs:719-746`). It is not a wire feature;
  it is a product catalog command that happens to cause the connection to emit
  server-initiated frames afterward. The client SDK exposes it as `subscribe_push`
  (analysis §1.3 feature matrix).
- **PUSH-002** — **Frame shape.** Each pushed frame is a normal `Response` whose `id`
  is the reserved `PUSH_ID = u32::MAX` and whose `result` is `Ok(Value)`
  (`synap/.../server.rs:295-298`, wired at `:264-308`). On the wire that is exactly
  `Response{id: 4294967295, result: Ok(<Value>)}` — identical framing to any other
  response, pinned by the corpus vector `push-id-u32-max.yaml`
  (`ok: ["message","news","hello"]` as a concrete example payload).
- **PUSH-003** — **Payload is a bare `Value`.** The `Ok` value is an arbitrary
  product-shaped `Value` — Synap chooses it per subscription; there is no wrapper. The
  client's push hook receives the decoded `Value` verbatim (SPEC-003 CLT-060: "the
  handler receives the decoded `Value`").
- **PUSH-004** — **Emission lifetime.** The server's dedicated writer task owns the
  write half for the whole connection lifetime (SPEC-004 SRV-002), so push frames may
  be emitted long after the `SUBSCRIBE` response and interleaved with normal responses
  — "subscription-style flows (Synap `SUBSCRIBE`) can emit long after" (SPEC-004
  commentary at SRV-002). One serialization per frame still holds (SRV-007).

## The contract Thunder already froze around it

- **PUSH-005** — **`PUSH_ID` is reserved, both directions.** Clients MUST NOT use
  `u32::MAX` as a request id and servers MUST refuse requests carrying it; client
  demultiplexers MUST route it distinctly (WIRE-005, CLT-010, CLT-060). The Rust and
  Go clients implement this today: the id allocator skips `PUSH_ID`
  (`rust/.../client/conn.rs:477-481`), and `id == PUSH_ID` routes to the hook rather
  than matching a pending call (`conn.rs:408-415`; Go `go/client` `OnPush`).
- **PUSH-006** — **Profile-gated.** Under `push = Reserved` (the standard) receiving a
  push frame is a protocol error that poisons the connection (CLT-060, CLT-014); only
  under `push = Enabled` is it delivered to the hook (SPEC-002 PRO-001, PRO-031). Both
  Rust and Go prove both branches (`behavior_test.go`
  `TestPushFramesRouteToHandlerUnderEnabled` / `TestPushUnderReservedPoisonsConnection`).

## What the baseline does NOT carry

These are the gaps §2 turns into requirements. None is a defect in `SUBSCRIBE` — they
are simply capabilities a single-channel pub/sub never needed:

- **PUSH-007** — **No event type.** Every frame is an undistinguished `Ok(Value)`. A
  consumer cannot tell a data event from a progress tick from a terminal error without
  the product baking a discriminator into its own payload — i.e. each product
  reinvents one. That reinvention is the drift this design exists to pre-empt.
- **PUSH-008** — **No subscription correlation.** All frames on a connection share the
  one id `PUSH_ID`. With two concurrent subscriptions on one connection there is no
  wire-level way to say which frame belongs to which — Synap's single-subscription
  usage sidesteps it, but watch/progress with many concurrent streams cannot.
- **PUSH-009** — **No stream termination.** There is no "this stream is done" signal;
  a client cannot free per-subscription state except by tearing down the connection or
  by a product-specific sentinel value (drift again).

**Baseline verdict**: `SUBSCRIBE` is a correct single-channel, untyped, unterminated
push. The design in §3 keeps that byte-for-byte as `push = Enabled` and adds the three
missing capabilities as an opt-in envelope under a new `push = Streaming` setting, so
Synap need not change a byte to keep working — and *may* adopt the envelope later
purely as an internal upgrade (see [05](05-coordination-and-open-questions.md)).
