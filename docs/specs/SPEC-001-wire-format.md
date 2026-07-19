# SPEC-001 — Wire Format Binding

| | |
|---|---|
| **Status** | Draft — the underlying wire is **already frozen** (family v1); this spec binds Thunder to it and adds canonicalization rules |
| **Phase / tasks** | Phase 0 · T0.3 + Phase 1 · T1.1 ([DAG](../DAG.md)) |
| **PRD requirements** | FR-01..FR-06; NFR-01, NFR-02 |
| **Requirement prefix** | `WIRE-` |
| **Source** | `Nexus/docs/specs/rpc-wire-format.md` v1 (canonical); `Vectorizer/docs/specs/VECTORIZER_RPC.md` (golden vectors); analysis [§1](../analysis/01-current-state.md), T-005 |

Requirement IDs `WIRE-xxx`. The normative byte definition lives in [`docs/spec/`](../spec/)
(transplanted verbatim at T0.3); this spec does not restate it — it binds Thunder's implementations
to it and resolves the drift the analysis found.

---

## 1. Binding to the frozen v1

- **WIRE-001** [P0] Every Thunder implementation SHALL produce and consume the family wire v1
  exactly: frame = `u32 LE length` (body bytes only) + MessagePack body; body = `Request{id: u32,
  command: String, args: [Value]}` or `Response{id: u32, result: Result<Value, String>}` in
  rmp-serde's default externally-tagged representation.
- **WIRE-002** [P0] The value model SHALL be exactly the 8 variants
  `Null | Bool | Int(i64) | Float(f64) | Bytes | Str | Array | Map`, where `Map` is an ordered
  list of `[key, value]` pairs and keys MAY be any value.
- **WIRE-003** [P0] Externally-tagged forms SHALL match the reference: unit variant as bare string
  (`"Null"`), payload variants as single-key maps (`{"Int": 42}`), and `Response.result` as the
  nested `{"Ok": <value>}` / `{"Err": <string>}` — including the double-nesting
  `{"Ok": {"Str": "PONG"}}` pinned by the corpus.
- **WIRE-004** [P0] The wire version is `1` and SHALL NOT change. Adding commands, profile fields
  with defaults, or new language ports MUST NOT bump it (PRD NFR-01).
- **WIRE-005** [P0] `PUSH_ID = u32::MAX` is reserved for server-initiated frames. Clients MUST NOT
  use it as a request id; servers MUST refuse requests carrying it; client demultiplexers MUST
  route it distinctly (SPEC-003 CLT-060).

## 2. Canonical encoding rules (drift resolution)

These resolve the live divergences found in analysis T-005. "Canonical" = what Thunder emits;
tolerated legacy forms are decode-only, forever within 1.x.

- **WIRE-010** [P0] **`Bytes` SHALL be emitted as MessagePack `bin`** (bin8/16/32). In Rust this
  requires a `serialize_bytes` path (e.g. `serde_bytes`) — the plain-`Vec<u8>`-as-seq form is
  non-canonical.
- **WIRE-011** [P0] Decoders SHALL **accept** `Bytes` arriving as a MessagePack array of integers
  0–255 (Synap ≤1.x legacy) and normalize it to the `Bytes` variant. Emitting this form is
  forbidden.
- **WIRE-012** [P0] **`Request` and `Response` SHALL be emitted as array-encoded structs**
  (`[id, command, args]` / `[id, result]` — the rmp-serde default).
- **WIRE-013** [P0] Server-side decoders SHALL **accept** map-shaped `Request`
  (`{"id":…, "command":…, "args":…}` — Synap Python/Go/Java ≤1.x legacy). Client-side decoders
  MAY reject map-shaped `Response` (no family SDK ever emitted one).
- **WIRE-014** [P0] Integers SHALL be packed in the shortest MessagePack form (compact ints);
  floats SHALL be packed as f64 preserving bit patterns (NaN payload round-trips; corpus-pinned).
- **WIRE-015** [P0] Strings are UTF-8 `str` family; `Bytes` is never used to smuggle text and
  `Str` never to smuggle binary. Empty `Bytes`/`Str`/`Array`/`Map` are legal and corpus-pinned.
- **WIRE-016** [P1] Removal of a decode tolerance (WIRE-011, WIRE-013) is a **major** version
  event and requires every family server ≥ the version that stopped emitting the legacy form.

## 3. Frame safety

- **WIRE-020** [P0] The frame cap SHALL be configurable per connection/profile with default
  **64 MiB**, and SHALL be validated against the length prefix **before any body allocation**, in
  every language, on both encode and decode (analysis T-004).
- **WIRE-021** [P0] A frame exceeding the cap SHALL produce a distinct, typed error
  (`FrameTooLarge`-class) without allocating the body buffer; the corpus contains a cap+1 vector
  asserting rejection-without-allocation semantics.
- **WIRE-022** [P0] Decoders SHALL handle partial input without error (return "need more bytes"),
  consume exactly one frame per decode, and support multiple frames buffered back-to-back.
- **WIRE-023** [P0] A malformed body (valid length, garbage MessagePack) SHALL produce a typed
  decode error and MUST NOT panic/throw uncontrolled; the connection-level policy on decode errors
  belongs to SPEC-003/SPEC-004.
- **WIRE-024** [P0] A frame whose length prefix is **zero** SHALL be a **valid frame carrying no
  body** — a keep-alive / liveness tick — not a malformed frame.
  - A **raw** (borrowed-body) decode SHALL return it as an empty body, so a consumer can skip it.
  - A **typed** decode, which by definition needs a body to deserialize, SHALL reject it with a
    decode error **distinct from the malformed-body error of WIRE-023**, so callers match on
    intent rather than on a parse failure.
  - Emission is not mandated: a peer MAY send zero-length frames as liveness ticks; no peer is
    required to.

  *Rationale.* This was previously undefined, so each product decided for itself and worked around
  the others: Fluxum uses zero-length frames as idle liveness on its push stream and had to wrap
  the TypeScript `FrameReader` to consume them before delegating (GH #6). Defining it once removes
  the workaround and stops the next product reinventing it. No byte changes — a zero-length frame
  was always representable; only its *meaning* was missing.

## 4. Purity

- **WIRE-030** [P0] The wire layer SHALL be pure in every language: no sockets, no timers, no
  product knowledge, no profile dependency. In Rust the `thunder::wire` layer SHALL NOT depend on
  tokio — `thunder` with `default-features = false` compiles the wire layer alone, with tokio pulled
  in only by the `client`/`server` features (PRD NFR-09); TS/Python/C# wire modules operate on byte
  buffers only.
- **WIRE-031** [P0] Serialization libraries are fixed per language (analysis T-011): Rust
  `rmp-serde` 1.x · TypeScript `@msgpack/msgpack` ^3 · Python `msgpack` ≥1.1
  (`use_bin_type=True`) · C# `MessagePack` 2.5.x via low-level `MessagePackWriter`/`Reader` ·
  Go `vmihailenco/msgpack` v5 with `UseCompactInts(true)` · PHP `rybakit/msgpack` ^0.9 driven at
  the low level (`packArrayHeader`/`packMapHeader`/`packBin`, never the generic `pack()` for
  values, which cannot distinguish `Bytes` from `Str`).
  `MessagePackSerializer.Typeless` and hand-rolled MessagePack codecs are **forbidden** (PRD NFR-02).

## 5. Error string conventions (v1)

- **WIRE-040** [P0] `Response.result` errors are strings. Thunder SHALL preserve them verbatim on
  the wire; interpretation (prefix parsing) is a client concern driven by the profile
  (SPEC-002 PRO-014, SPEC-003 CLT-050). Thunder never invents a third convention: the family's two
  (`ERR`/`NOAUTH`/`WRONGPASS` prefixes; `"[code] message"`) are the only ones modeled.
