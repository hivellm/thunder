# SPEC-008 — Canonical Behavior

| | |
|---|---|
| **Status** | Draft — canonical behavior freezes at G1; TLS/handshake ship as optional capabilities post-1.0 |
| **Phase / tasks** | Phase 6 · `phase6_canonical-behavior-spec` ([DAG](../DAG.md)) |
| **PRD requirements** | NFR-01, FR-10..FR-12, FR-29 |
| **Requirement prefix** | `CAN-` |
| **Source** | Behavioral-normalization analysis [BN-001..BN-023](../analysis/behavioral-normalization/), the agnostic-config refactor (`Config::standard()`), and the decisions in [`.rulebook/decisions/`](../../.rulebook/decisions/) |

---

Thunder's feasibility work established **"profiles, not forks"**: one codec, one client contract,
per-product behavior parameterized as config. The behavioral-normalization analysis (BN-001..BN-023)
went one step further and showed those per-product rows converge to a **single canonical behavior**.
The agnostic-config refactor then ratified the mechanism: there is exactly **one** standard
(`Config::standard()`, SPEC-002 PRO-011), and every product is that standard plus its own identity
and optional capabilities — no product names in the library, no registry to flip.

This spec is the normative statement of **what that one behavior is**, per dimension, so the
implementation tasks (`phase6_tls-optional`, `phase6_handshake-optional`, `phase6_cheap-convergence`)
build against a pinned definition instead of re-deriving it. It does not introduce new wire bytes
(SPEC-001 is frozen) and it does not restate decisions already normative elsewhere — it **cites**
them and pins the one dimension still open: the canonical error-grammar spelling (§4).

> **The governing principle — shape ≠ policy.** Every dimension below is normalized as a *shape* or
> a *default*, never as a mandate that changes whether a deployment authenticates or encrypts. A
> HELLO handshake is about frame ordering and capability negotiation; it is **not** a demand for
> credentials. TLS is an available transport option, not a requirement. Normalizing the family's
> behavior must never silently turn an open deployment into a closed one — that would be a security
> regression, and it was never the goal (SPEC-002 PRO-001a, SPEC-004 SRV-011).

## 1. The convergence principle

- **CAN-001** [P0] The family's canonical behavior SHALL be `Config::standard()` (SPEC-002 PRO-011),
  pinned as language-neutral data in `conformance/standard.yaml`. A conforming product differs from
  the standard only by its **identity** (`scheme`, `default_port`) and any **optional capability** it
  turns on (§2 handshake variant, §3 TLS, §6 push emission). Convergence is therefore visible and
  per-application: delete overrides until only identity remains (SPEC-002 PRO-022).
- **CAN-002** [P0] **Shape is not policy.** A behavioral dimension MUST NOT be normalized in a way
  that changes a deployment's security posture. The canonical HELLO shape (§2) does not require
  credentials; the canonical error grammar (§4) does not require auth; TLS (§3) is off by default.
  Auth *enforcement* and TLS *on/off* remain per-deployment policy — this is the normative home of
  SPEC-002 PRO-001a and SPEC-004 SRV-011, which this spec governs but does not supersede.

## 2. Handshake

- **CAN-010** [P0] The canonical handshake **shape** SHALL be a leading `HELLO` frame carrying a Map
  `{version, client_name, and optionally token | api_key | [user, pass]}`, answered by a Map
  `{protocol_version (proto), capabilities, …}` — the `HelloMandatory` + `MapPayload` standard
  (SPEC-002 PRO-011, WIRE-004; corpus `handshake-map-hello-request`,
  `handshake-capabilities-hello-reply`). It is the only handshake that negotiates `proto` and
  advertises capabilities, which is what an evolving protocol needs.
- **CAN-011** [P0] The canonical HELLO SHALL be an **optional, opt-in capability**, not a mandate.
  Every server can offer it and every client can speak it, but a deployment MAY configure a different
  first-frame shape (`AuthCommand` with `hello_style: NotUsed` or `ArgLess`) without ceasing to
  conform. Whether the first HELLO is *required* is `HelloMandatory` vs `AuthCommand` (a config,
  SPEC-002 PRO-030); whether credentials are *enforced* is a separate deployment toggle
  (`auth_required`, PRO-001a) — the two MUST NOT be conflated (CAN-002).
- **CAN-012** [P0] A server SHALL **dual-accept**: it answers the canonical HELLO **or** its
  configured non-standard first frame, disambiguated by the first frame's command and the profile's
  `handshake`/`hello_style` (SPEC-004 SRV-011). The non-standard first-frame shapes — no-HELLO +
  bare `AUTH` (the `AuthCommand`/`NotUsed` config) and the arg-less `HELLO` (the `AuthCommand`/
  `ArgLess` config) — are **supported configurations, not deprecated legacy**: a client so
  configured emits them and a server so configured accepts them. They are pinned by the corpus
  vectors `handshake-argless-hello-request` and `handshake-metadata-hello-reply` as live, emittable
  shapes (mode `bidirectional`), not decode-only tolerances.

## 3. TLS

- **CAN-020** [P1] The family SHALL ship **one** optional TLS layer: `tokio-rustls`, config-gated
  (`tls.cert_path` / `tls.key_path`), feature-gated in the crate (`tls`), **no STARTTLS**, **off by
  default** (SPEC-004 SRV-040, PRD FR-29). It is available to every configuration and, when off, the
  documented posture is the family's loopback/private bind. An off-by-default encrypted-transport
  option cannot break a deployed plaintext client — TLS is therefore purely additive (BN-009). No
  HiveLLM product runs RPC TLS today (BN-007, and the SRV-040 ordering decision in
  [`.rulebook/decisions/2026-07-17-registry-names.md`](../../.rulebook/decisions/2026-07-17-registry-names.md)),
  so this is the family's *first* running RPC TLS, implemented once here rather than per product.

## 4. Error grammar

- **CAN-030** [P0] The canonical error grammar SHALL be the **`both` superset** (SPEC-002 PRO-011,
  `error_codes: both`): a server MAY emit a bracketed `"[<CODE>] <message>"` for a product-specific
  code, **and** the auth-family tokens are emitted **bare** in their RESP3 spelling — `NOAUTH`,
  `WRONGPASS`, `NOPERM`, `NOPROTO` — as `"<TOKEN> <message>"`. This is the pinned decision on the one
  spelling the analysis left open (BN-011); rationale and the rejected alternative are recorded in
  [`.rulebook/decisions/2026-07-17-canonical-error-grammar.md`](../../.rulebook/decisions/2026-07-17-canonical-error-grammar.md).
- **CAN-031** [P0] The **auth-class** token set SHALL be the closed set `{NOAUTH, WRONGPASS, NOPERM}`.
  Every client on the `both` grammar MUST map a leading token from this set to the **auth class**, and
  MUST extract a leading `"[code] "` into a structured `code`, regardless of which form a given server
  emits (SPEC-003 CLT-050/CLT-051). All four language clients recognize exactly this set. `NOPERM` —
  an authorization refusal, classed with authentication because the client's recourse is the same
  (present different credentials) — was unmodeled until the BN-023 errata and is corpus-pinned by
  `response-err-noperm`. `NOPROTO` (protocol-version negotiation failure) is a reserved token of the
  convention that no RPC path emits today; it is **not** auth-class (its recourse is a version
  change, not credentials), so no client special-cases it — an emitted `NOPROTO` would classify as a
  generic server error until a path that emits it, and a rule for it, exist.
- **CAN-032** [P0] The two prior per-product grammars — RESP3-only (bare tokens) and bracket-only —
  are **subsets** of `both`. A client on `both` therefore reads every family server with no
  negotiation, and **no product must change its emission**: the bare auth tokens it emits today
  (Synap `NOAUTH`/`WRONGPASS`/`NOPERM`, Nexus `WRONGPASS`) are already canonical. Assertions branch
  on class and `code`, never on message text (CLT-052).

## 5. Caps

- **CAN-040** [P0] The canonical frame cap SHALL be a **64 MiB** default, **checked before the body
  is allocated** (SPEC-001 WIRE-020), and operator-configurable per deployment (`max_frame_bytes`).
  A deployment MAY raise it (Synap's historical 512 MiB is such an override, not a dialect); the cap
  is config, never wire (SPEC-002 PRO-003). `max_in_flight` SHALL be a single configurable default
  (256, the family plurality); a larger value (Nexus's 1024) is a config value, not an identity.

## 6. Push

- **CAN-050** [P0] `PUSH_ID` SHALL be a **server→client-only** channel: a client MUST NOT send it as
  a request id, a server MUST NOT accept it as one, and **every** client MUST route an inbound
  `PUSH_ID` frame to a registered push hook (SPEC-003 CLT-060). The wire reservation is already
  uniform. Whether a given server *emits* a push frame is a property of its command catalog — the
  `Reserved` vs `Enabled` config — a **per-product capability, not a dialect** (BN-010).

## 7. Conformance

- **CAN-090** [P0] Every canonical behavior in this spec MUST map to at least one corpus vector or
  behavioral-floor test (SPEC-005 TST-090). The canonical shapes are pinned as `bidirectional`
  vectors (encode **and** decode byte-exact); the genuinely deprecated forms that Thunder decodes but
  never emits — the WIRE-011/016 legacy tolerances (Synap int-array `Bytes`, map-shaped `Request`) —
  stay `decode-only` (`encode(decoded) != frame`). Non-standard *config* shapes (§2 CAN-012) are
  **not** legacy and remain `bidirectional`: they are emitted by a client so configured. The error
  grammar is pinned by `response-err-bracket-code`, `response-err-noauth`, `response-err-wrongpass`,
  and `response-err-noperm` (CAN-031).

## 8. Optionality summary

| Dimension | Canonical behavior | Optionality |
|---|---|---|
| Handshake (§2) | leading HELLO map + capabilities reply + `proto` | **optional capability** — every server offers, every client speaks; a config MAY use another first-frame shape; enforcement is a separate deployment toggle |
| TLS (§3) | one `tokio-rustls` layer, no STARTTLS | **optional, off by default** — available to every config, opt-in per deployment |
| Error grammar (§4) | `both` superset: bracketed `[CODE]` + bare auth tokens | **canonical** — clients parse both; servers need not change emission |
| Caps (§5) | 64 MiB pre-alloc, `max_in_flight` 256 | **canonical defaults, per-deployment configurable** |
| Push (§6) | `PUSH_ID` server→client-only, uniform client hook | **canonical**; whether a server *emits* is a per-product capability |
