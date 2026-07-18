# §4 — Draft PRO-001 `push` field evolution (task 1.5)

**Proposed** evolution of the `push` dimension in SPEC-002 PRO-001. Draft for
coordination, not a spec edit. The design rule the whole config model rests on:
**adding a field or an enum case is a minor release because defaults keep every existing
data file and application valid** (PRO-002, PRO-022, PRO-003). This evolution obeys it
exactly.

## Today

PRO-001 `push`:

| Field | Values | Semantics |
|---|---|---|
| `push` | `Reserved` \| `Enabled` | `Reserved`: server refuses client `PUSH_ID`, never emits push — **the standard**. `Enabled`: push frames delivered to the client hook (an application shipping a subscribe-style command). |

Standard value: `Reserved` (PRO §3 table). Emission is the product's job (PRO-031).

## Proposed

- **PUSH-030** — **Add one enum case: `Streaming`.**

  | Field | Values | Semantics |
  |---|---|---|
  | `push` | `Reserved` \| `Enabled` \| `Streaming` | `Reserved`: unchanged — server refuses client `PUSH_ID`, never emits push (**the standard**). `Enabled`: unchanged — push frames deliver a **bare `Value`** to the client hook (Synap `SUBSCRIBE` today). `Streaming`: push frames carry the **canonical envelope** `{stream, kind, data}` (SPEC-001 §push draft, WIRE-050); the client routes by `stream`, dispatches on `kind`, and honors terminal kinds. |

- **PUSH-031** — **`Streaming` is a strict superset of `Enabled`.** The wire framing is
  identical (`Response{id: PUSH_ID, result: Ok(Value)}`); the only difference is that a
  `Streaming` client interprets the `Ok` value as the envelope rather than delivering it
  verbatim. Anything `Enabled` can express, `Streaming` can (a single `data`-kind stream
  ≡ `SUBSCRIBE`). This is why it is a new case and not a breaking change to `Enabled`.

- **PUSH-032** — **Default unchanged, so it is a minor release.** The standard stays
  `push = Reserved` (PRO §3). Every existing profile that set `Reserved` or `Enabled`
  keeps its exact value and behavior; no data file is invalidated (PRO-002). Adding the
  case is the "adding a dimension is minor" rule (PRO-022) applied to a dimension's value
  set.

- **PUSH-033** — **Still data, not behavior (PRO-003).** `push = Streaming` changes
  **no wire bytes** relative to `Enabled` — same frame, same `PUSH_ID`, same MessagePack
  `Map`. It only selects a client-side interpretation. So it does not violate PRO-003's
  "no config may alter wire bytes": the bytes a `Streaming` server emits are a
  well-formed `Enabled` payload that happens to follow the envelope shape; a `Streaming`
  client is one that *relies* on that shape. The shape lives in SPEC-001 §push, the
  toggle in the profile.

- **PUSH-034** — **Server-side meaning (PRO-031 extension).** `Reserved` servers refuse
  client `PUSH_ID` and never emit (unchanged). `Enabled` and `Streaming` both delegate
  emission to the product dispatch layer; a `Streaming` server's emit API produces
  enveloped frames (SPEC-004 SRV forward-look, [03](03-spec-001-push-amendment.md)),
  an `Enabled` server's produces bare values. A deployment moving from `Enabled` to
  `Streaming` is an application decision, adopted when its clients understand the
  envelope.

## Migration for the one existing producer (Synap)

- **PUSH-035** — **Synap needs no change to keep working.** It stays `push = Enabled`
  and its bare-`Value` `SUBSCRIBE` is untouched. Adopting `Streaming` is a *later,
  optional* internal upgrade: wrap the emitted value in a single-`data`-kind envelope
  and set the profile to `Streaming`. Because that is a wire-compatible payload-shape
  change gated by the profile, it can be rolled out with the usual dual-accept caution
  (a `Streaming` client tolerating a transition), coordinated with Synap — see
  [05](05-coordination-and-open-questions.md). This proposal does **not** require it.

## Standard-profile table (PRO §3) — proposed row

Unchanged value, clarified rationale:

| Dimension | Standard | Why this value |
|---|---|---|
| `push` | `Reserved` | `PUSH_ID` is server→client only; *emitting* is a capability an application opts into (`Enabled` bare, or `Streaming` enveloped). |
