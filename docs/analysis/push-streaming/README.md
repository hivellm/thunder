# Push / Streaming v-next — Family Design Proposal

> **Status**: **Proposal-stage** (design only, no implementation). Task
> `phase5_push-streaming-design` (T5.2). Governing specs: SPEC-001 §push (WIRE-005),
> SPEC-002 (PRO-001 `push` field). P2, post-1.0 fast-follow.
>
> **Question**: The family ships server-initiated frames in exactly one place —
> Synap's `SUBSCRIBE` over the reserved `PUSH_ID`. Other products will want more:
> watch, progress, cache invalidation. Can one family-level design cover them,
> **wire-compatible via the reserved `PUSH_ID` so the wire version stays `1`**
> (WIRE-004), instead of each product inventing its own — the exact drift Thunder
> exists to end?
>
> **Answer (proposed)**: Yes, and it costs **zero wire bytes of new framing**. Every
> server-initiated frame already is `Response{id: PUSH_ID, result: Ok(Value)}`. The
> only thing missing to carry watch/progress/invalidation over *one* connection is a
> **canonical envelope inside that `Value`** (`{stream, kind, data}`), plus a
> backward-compatible third setting of the `push` profile field that tells the client
> to interpret the payload as that envelope. Synap's bare-`Value` `Enabled` behavior
> is frozen and untouched; the new `Streaming` setting is a strict superset.

## Why this is a proposal, not a change

The wire is frozen (WIRE-004) and the corpus is normative across five languages. This
task's deliverable is a **spec-amendment proposal + proposal-stage corpus vectors**,
not an edit to the frozen SPEC-001/SPEC-002 text and not code. Two of its steps —
coordinating the envelope with Synap (the only shipped push producer) and ratifying
the `push`-field evolution — are owner/cross-repo decisions and are called out as
**open** in [05](05-coordination-and-open-questions.md). Nothing here is normative
until that coordination lands and the vectors are promoted from
`conformance/vectors/proposal-push-streaming/` into the normative corpus.

## Section index

| § | File | Contents |
|---|---|---|
| §1 | [01-synap-subscribe-baseline.md](01-synap-subscribe-baseline.md) | The one shipped push producer: Synap `SUBSCRIBE` — frame shape, delivery guarantees, what it does and does not carry today (task 1.1) |
| §2 | [02-family-needs.md](02-family-needs.md) | Push/streaming needs beyond `SUBSCRIBE`: watch, progress, invalidation — collected across Nexus/Vectorizer/Lexum (task 1.2) |
| §3 | [03-spec-001-push-amendment.md](03-spec-001-push-amendment.md) | Draft SPEC-001 §push: the canonical push envelope over `PUSH_ID`, no wire-version bump; non-goals (tasks 1.4, 1.7) |
| §4 | [04-pro-001-push-field-evolution.md](04-pro-001-push-field-evolution.md) | Draft PRO-001 `push` field evolution: `Reserved \| Enabled \| Streaming` with backward-compatible defaults (task 1.5) |
| §5 | [05-coordination-and-open-questions.md](05-coordination-and-open-questions.md) | What must be agreed with Synap before ratification; open questions; promotion checklist (task 1.3) |

Findings are numbered **PUSH-001..PUSH-0xx** globally across these files.

## Executive summary

**One frame shape, already in the corpus.** Every server-initiated frame is
`Response{id: PUSH_ID, result: Ok(Value)}` — pinned by `push-id-u32-max.yaml`. The
client demultiplexer routes `id == PUSH_ID` to a registered hook and hands it the
decoded `Value` (SPEC-003 CLT-060). Synap's `SUBSCRIBE` emits an arbitrary `Value`
(e.g. `["message","news","hello"]`); there is no event type, no subscription
correlation, and no stream termination signal — a single logical channel per
connection (PUSH-001..PUSH-004).

**The gap is semantics, not bytes.** watch/progress/invalidation need three things
`SUBSCRIBE` lacks: (a) **which** logical stream a frame belongs to, so many
subscriptions can share one connection; (b) an **event kind**, so `data` /
`progress` / `end` / `error` / `invalidate` are distinguishable without sniffing the
payload; (c) an explicit **end** so a client can free a stream (PUSH-010..PUSH-013).

**The proposal: a canonical envelope inside the existing `Value`** (PUSH-020):

```
result: Ok(Map{
  "stream": Int,   # u32; correlates to the Request id that opened the stream; 0 = uncorrelated broadcast
  "kind":   Str,   # open vocabulary: "data" | "progress" | "end" | "error" | "invalidate" | <product-defined>
  "data":   Value  # opaque to Thunder — product payload (WIRE-030 purity holds)
})
```

Because this only changes the *shape of the `Value`*, **not the framing**, WIRE-004
(no version bump) and WIRE-005 (`PUSH_ID` reserved) are untouched. It rides the exact
bytes the five languages already round-trip — the proposal-stage vectors prove it by
passing every implementation's wire codec unchanged.

**The client learns to interpret it from the profile, not the bytes** (PUSH-030). The
`push` field grows `Reserved | Enabled → Reserved | Enabled | Streaming`. `Enabled`
stays exactly Synap's bare-`Value` behavior (frozen for compatibility); `Streaming` is
a strict superset that routes by `stream` and surfaces `kind`. Adding an enum case is a
**minor** release (PRO-002/PRO-022): existing data files and applications keep their
`Reserved`/`Enabled` values and behavior.

**Non-goals** (PUSH-040): no implementation in this task; **chunked streaming**
(fragmenting one large value across frames with flow-control windows) stays **v2**;
no backpressure/credit protocol this round — the product governs emission rate,
Thunder does not negotiate windows; no new command — `SUBSCRIBE`/`WATCH`/etc. remain
product catalog commands, Thunder standardizes only the push envelope they emit.
