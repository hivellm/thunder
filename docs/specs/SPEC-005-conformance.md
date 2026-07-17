# SPEC-005 — Conformance & Testing

| | |
|---|---|
| **Status** | Draft — corpus format freezes at G1 |
| **Phase / tasks** | Phase 0 · T0.4 + Phase 1 · T1.2/T1.3 + every later phase (gates) ([DAG](../DAG.md)) |
| **PRD requirements** | FR-50..FR-54; NFR-03 |
| **Requirement prefix** | `TST-` |
| **Source** | Vectorizer golden vectors (`vectorizer-protocol/src/rpc_wire/`), Nexus round-trip matrix, analysis [§3 T-015..T-017](../analysis/03-conformance-and-versioning.md) |

Requirement IDs `TST-xxx`. The corpus is the product: byte-compatibility across four languages and
three server products becomes a CI property instead of a convention.

---

## 1. Corpus format

- **TST-001** [P0] Vectors live in `conformance/vectors/` as language-neutral data files (YAML),
  one vector per file, schema:

  ```yaml
  name: request-ping
  group: canonical            # canonical | value | framing | tolerance | push | handshake
  mode: bidirectional         # bidirectional | decode-only | reject
  frame_hex: "08 00 00 00 93 01 a4 50 49 4e 47 90"
  decoded:
    kind: request
    id: 1
    command: PING
    args: []
  notes: "VECTORIZER_RPC.md §11 canonical vector"
  ```

- **TST-002** [P0] `mode` semantics: `bidirectional` — decode(frame) == decoded **and**
  encode(decoded) == frame, byte-exact; `decode-only` — decode succeeds and equals `decoded`,
  encoding this form is forbidden (legacy tolerances WIRE-011/013); `reject` — decode MUST fail
  with the named error class (e.g. cap violation) and, for cap vectors, without allocating.
- **TST-003** [P0] The corpus format itself freezes at G1; adding vectors is a minor change,
  changing existing canonical bytes is forbidden (that would be a wire change — NFR-01).

## 2. Corpus contents (1.0 floor)

- **TST-010** [P0] **Canonical group**: PING request `08 00 00 00 93 01 a4 'PING' 90`; PONG
  response with nested `{"Ok":{"Str":"PONG"}}` (the two family-pinned vectors).
- **TST-011** [P0] **Value group**: every variant alone and nested; empty `Bytes`/`Str`/`Array`/
  `Map`; `Map` with non-string keys; `i64::MIN`/`MAX` and compact-int boundary values
  (−32, 127, 255, 65535, …); `f64` NaN bit pattern, ±∞, −0.0; `Err` plain, `Err` with
  `"[code] "` prefix, `NOAUTH`/`WRONGPASS` strings.
- **TST-012** [P0] **Framing group**: two frames in one buffer; partial header; partial body;
  zero-length-body frame; frame at exactly the cap; frame one byte over the cap (`reject`,
  no-allocation assertion).
- **TST-013** [P0] **Tolerance group** (`decode-only`): `Bytes` as int-array (Synap legacy);
  map-shaped `Request`.
- **TST-014** [P0] **Push group**: a frame with `id = u32::MAX` (routing assertion per profile).
- **TST-015** [P0] **Handshake group**: Nexus `HELLO [Int(1)]` request/reply shape; Vectorizer
  HELLO map (`version`/`token`/`api_key`/`client_name`) and capabilities reply.
- **TST-016** [P0] Every SPEC-001 MUST maps to ≥ 1 vector; the mapping is recorded in the vector's
  `notes` (reviewable coverage).

## 3. Loaders and gates

- **TST-020** [P0] Each language ships one corpus loader (~50 LOC) that walks
  `conformance/vectors/` and asserts per `mode`. The loader runs in the **default** test command
  of each package — never feature-gated, never ignored (PRD NFR-03; the anti-pattern is documented
  family history).
- **TST-021** [P0] CI runs the corpus in all four languages on every PR; a wire-affecting PR that
  passes in one language and fails in another cannot merge.

## 4. Reference cross-decode

- **TST-030** [P0] The Rust suite SHALL take `nexus-protocol` as a dev-dependency and assert both
  directions over the canonical + value groups: Thunder-encoded frames decode via
  `nexus_protocol::rpc` into equal structures, and vice versa. Thunder is pinned to the family's
  shipping reference, not to itself.
- **TST-031** [P1] When the terminal shims land (T2.4), the dev-dependency moves to the last
  pre-shim version, pinned — the reference must remain the *old* independent implementation.

## 5. Pairwise cross-language fuzz

- **TST-040** [P0] A seed generator (checked in, deterministic per seed) produces random `Value`
  trees as JSON; each language encodes its tree and every other language decodes it and re-encodes;
  all four byte outputs and all decoded trees must agree. Runs on a fixed seed set per PR and a
  rolling seed nightly.
- **TST-041** [P0] Divergence output includes the shortest failing tree (auto-shrunk) and lands as
  a new corpus vector once fixed — fuzz findings graduate into the permanent corpus.

## 6. Live interop

- **TST-050** [P1] Env-gated smoke (`THUNDER_LIVE_URL_SYNAP/NEXUS/VECTORIZER`): connect, handshake
  per profile, PING-class call, one typed-error call, clean close — per language. Release-path
  only, following the family's `s2s`/live-test gating convention.

## 7. Profile coverage

- **TST-060** [P0] Every registered profile (PRO-010) is exercised: handshake vectors +
  behavioral floor (CLT-090) run under each profile's constants. A registry data error fails CI in
  all languages simultaneously.

## 8. Traceability

- **TST-090** [P0] Requirement → test mapping is enforced by review checklist: a PR touching a
  `WIRE-`/`PRO-`/`CLT-`/`SRV-` MUST names the vectors/tests that cover it in its description.
  (Lightweight by design — the corpus `notes` field carries the durable mapping.)
