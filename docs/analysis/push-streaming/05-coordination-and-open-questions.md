# §5 — Coordination with Synap & open questions (task 1.3)

The proposal (§3, §4) is deliberately shaped so **nothing forces a change on the one
shipped producer**. But two things must be *agreed* before any of it becomes normative,
and both are owner/cross-repo decisions — they cannot be settled inside the Thunder repo
alone. This section records the agreement surface and the open questions, and gives the
promotion checklist that closes the task once agreement lands.

> **Status of task 1.3**: **OPEN — awaiting Synap coordination.** Synap is the only
> product emitting push frames; the envelope must be reviewed against its `SUBSCRIBE`
> internals and its SDK push hook before ratification. This is a human/product decision
> (per the product-swap policy, Thunder-repo work only) and is not resolved by this
> design pass. The design below is the *proposal to bring to that coordination*, not a
> decision taken without it.

## What must be agreed with Synap

- **PUSH-050** — **Envelope key names.** The draft uses descriptive keys `stream` /
  `kind` / `data`. The alternative is short keys (`s` / `k` / `d`) to shave ~12 bytes
  per push frame — cheap next to any real payload, but the project is byte-conscious
  (the shootout weighs bytes-on-wire). **Recommendation: descriptive keys** — push
  frames are control-plane, not the hot request path, and cross-language debuggability
  outweighs 12 bytes. Synap should confirm, since it owns the first implementation.
- **PUSH-051** — **Does `SUBSCRIBE` migrate or coexist?** Two viable end states:
  **(a)** `SUBSCRIBE` stays `Enabled` (bare value) forever and only new
  watch/progress/invalidation producers use `Streaming`; or **(b)** Synap upgrades
  `SUBSCRIBE` to emit a single-`data` envelope and moves to `Streaming` (PUSH-035),
  unifying the family on one shape. (a) is zero-effort; (b) is tidier long-term but is a
  Synap SDK/telemetry rollout. **This design supports both** — the choice is Synap's.
- **PUSH-052** — **Reserved `kind` vocabulary.** §3 reserves `data/progress/end/error/
  invalidate`. Synap (and Nexus/Vectorizer as the next adopters) should confirm these
  cover their cases and that none of their existing product payloads already means
  something incompatible under one of those names.
- **PUSH-053** — **Correlation via request-id reuse.** WIRE-051 sets `stream` = the
  opening `Request` id. Synap's `SUBSCRIBE` is effectively single-stream today so it
  never needed correlation; confirm that reusing the request-id space (rather than a
  separate subscription-id namespace) fits its planned multi-subscription usage.

## Open questions (design-level, not blocking the proposal)

- **PUSH-060** — **Late frames after `end`.** WIRE-054 lets a client ignore or reject
  frames for a closed stream. Should a *server* re-emitting on a closed stream be a
  protocol error (poison) or silently dropped client-side? Leaning: client-side ignore
  (robustness), server-side "SHALL NOT" (correctness) — as drafted. Confirm at
  ratification.
- **PUSH-061** — **Broadcast delivery scope.** WIRE-052's `stream = 0` says "every
  consumer" — but consumer of *what*? Per-connection (any client with `push =
  Streaming`) is the simplest and is what the draft assumes. A topic-scoped broadcast
  (only subscribers of topic X) would need a subscription registry, which edges toward
  the flow-control surface that is explicitly out (PUSH-016). Keep broadcast
  connection-scoped for v-next; topics are a product concern layered in `data`.
- **PUSH-062** — **Multiple products, one corpus.** When Nexus/Vectorizer adopt
  `Streaming`, their enveloped frames must round-trip identically. The proposal-stage
  vectors already assert the envelope is wire-neutral (any `Map`), so cross-product
  conformance is automatic at the wire layer; product-level `kind`/`data` semantics are
  validated per product, not in Thunder's corpus.

## Promotion checklist (closes the task's normative half, post-coordination)

When PUSH-050..PUSH-053 are agreed with Synap, ratification is mechanical:

1. Fold §3 (WIRE-050..056) into SPEC-001 as its §push section; add CLT-062..064 to
   SPEC-003 and SRV-011..012 to SPEC-004 (the informative sketches in §3).
2. Fold §4 (`push = Streaming`) into SPEC-002 PRO-001 and the PRO §3 standard table row.
3. Move the vectors from `conformance/vectors/proposal-push-streaming/` into
   `conformance/vectors/` (they already pass every language's wire codec — see that
   directory's README) and raise the anti-shrink floor accordingly.
4. Open the implementation task (client envelope parsing + server emit API + tests).

Until step 1 begins, this directory is the whole deliverable and everything in it is
proposal-stage.
