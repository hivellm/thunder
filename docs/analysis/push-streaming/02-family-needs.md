# §2 — Push/streaming needs beyond `SUBSCRIBE` (task 1.2)

Candidate server-initiated use cases across the family, each reduced to the wire-level
capability it needs. The point is not to enumerate product features — it is to find
the **smallest common shape** that serves all of them over one connection without a
wire-version bump. Sources: the family roadmap (`docs/ROADMAP.md` "Push/streaming
v-next"), the adoption plan (`docs/analysis/04-adoption-plan.md`), and the
behavioral-normalization inventory (which established push emission as the *only* real
per-product push difference — a capability, not a dialect).

## Use cases and the capability each demands

- **PUSH-010** — **Watch (Nexus graph, Vectorizer collections).** A client subscribes
  to a resource and receives a frame per change (node/edge upserted or deleted;
  collection mutated). Needs: **(a)** many concurrent watches on one connection —
  hence **subscription correlation** (PUSH-008); **(b)** a **`data`** event kind
  distinct from control frames; **(c)** an **`end`** when the watch is cancelled or
  the resource disappears (PUSH-009).
- **PUSH-011** — **Progress (Vectorizer indexing/ingest, Lexum long jobs).** A long
  operation streams incremental progress until it completes. Needs: a **`progress`**
  event kind carrying an opaque `{done, total, stage}`-shaped payload; a terminal
  **`end`** on success and a terminal **`error`** on failure — so the client resolves
  the job exactly once. Correlation ties the ticks to the specific job the client
  started.
- **PUSH-012** — **Cache invalidation (any product fronting a cache).** The server
  tells clients that keys are stale. Needs: an **`invalidate`** event kind carrying
  the affected keys/reason; typically **uncorrelated** (a broadcast to every subscriber,
  not tied to one request) — hence a `stream = 0` broadcast lane (PUSH-020).
- **PUSH-013** — **Synap `SUBSCRIBE` (today).** A single logical channel of `data`
  events. Falls out as the degenerate case: one stream, only the `data` kind, no
  mandatory `end`. It must keep working with **zero** change (PUSH-006), which is why
  `Enabled` is frozen and `Streaming` is additive.

## The common shape

Reading the four rows together, the union of what they need is exactly three fields
beyond the payload itself:

| Capability | Needed by | Field |
|---|---|---|
| Which logical stream this frame belongs to | watch, progress (many concurrent) | `stream` |
| What kind of event this is | all (data/progress/end/error/invalidate) | `kind` |
| The product-specific event body | all | `data` |

- **PUSH-014** — **No use case needs new *framing*.** Every row is expressible as one
  `Response{id: PUSH_ID, result: Ok(<envelope>)}` where the envelope is a `Map` of the
  three fields. Nothing needs a second frame type, a new value variant, or a wire
  version bump — which is the whole reason this can be a fast-follow rather than a v2.
- **PUSH-015** — **No use case needs Thunder to understand the payload.** `data` stays
  opaque: watch diffs, progress counters, invalidation key lists are all product
  shapes. Thunder routes by `stream` and surfaces `kind` as a string; it never parses
  `data` (WIRE-030 purity — the wire layer carries no product knowledge). This keeps
  the design inside the same purity boundary the rest of Thunder already honors.
- **PUSH-016** — **Backpressure is explicitly out (this round).** Progress and watch
  can be high-rate, but a credit/flow-control protocol is a much larger surface (window
  negotiation, per-stream buffering, cancellation races). The family's shipped producer
  has none and none of the four use cases is blocked without it: the product governs its
  own emission rate and the existing `max_frame_bytes` cap bounds any single frame. Flow
  control is deferred alongside chunked streaming (PUSH-040, [03](03-spec-001-push-amendment.md)).

**Needs verdict**: the four use cases collapse to one envelope of `{stream, kind,
data}`. §3 specifies it; §4 wires the profile so a client knows when to expect it.
