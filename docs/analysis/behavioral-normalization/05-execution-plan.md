# §5 — Execution Plan

> Phased and gated, layered onto Thunder's existing milestones (DAG P0–P5 / ROADMAP M0–M5) rather
> than replacing them. Normalization phases are labelled **N0–N4** to keep them distinct from
> Thunder's P/M numbering. Every phase ships standalone value; the hard handshake track (N3) can
> trail the cheap wins (N1) indefinitely without blocking them.

## 5.1 The ordering insight that makes this cheap

### BN-022 — Behavioral normalization is cheap only *after* Thunder consolidation; before it, the same convergence would mean editing 18 hand-ported transports instead of four servers + four profile rows

- **Evidence**: today the protocol lives in 18 independently maintained copies (analysis T-001);
  each of the four behavioral dimensions is re-decided per copy (three TS libs, three C# strategies,
  nine cap-less transports — T-003/T-004/T-007). Post-Thunder, every product's server runs
  `thunder::server` and every SDK runs a `thunder::client`, all driven by a profile value (SPEC-002,
  SPEC-004 SRV-011). A behavioral change becomes: a change in `thunder::server`'s dual-accept path
  (once), plus a per-product profile-value flip (four lines).
- **Impact**: this sequences the whole program. Attempting normalization *before* consolidation would
  multiply every convergence by 18 and re-introduce the drift Thunder exists to kill. Attempting it
  *after* makes handshake/error convergence a property of one shared server implementation plus a
  ledger of profile flips. **Normalization is therefore the natural sequel to Thunder M2/M3, not a
  parallel project** — it presupposes the products are already on Thunder. The cheap-win phase (N1)
  can begin as soon as a product swaps (M2 per product); the coordinated handshake track (N3) wants
  most products swapped first so the change surface is `thunder::server`, not each product's legacy
  listener.
- **Confidence**: high.

## 5.2 Phase N0 — Ratify the canonical behavior (decisions)

Depends on: Thunder M0 (spec home exists). Runs alongside/after Thunder M1.

0. **First, and independent of any normalization decision: fix the BN-023 registry errata** in one
   coordinated Thunder commit — `synap.handshake` → `auth_command`, `nexus.hello_style` →
   arg-less/metadata (and the `handshake-nexus-hello-request.yaml` corpus vector), `vectorizer.tls`
   → `reserved_config`, add `NOPERM` to the recognized auth tokens — touching the profile YAMLs,
   SPEC-002 PRO-001 prose, and the `Profile` constants + pinning tests in all four language
   packages together (sequenced after the in-flight T3 packages land). This is registry
   correctness, owed even if N1–N4 never run.
1. Write a normative **Canonical Behavior** spec (extend SPEC-002, or a new `SPEC-008-canonical-behavior.md`)
   pinning, for each dimension, the single family behavior from §2:
   - **Handshake shape** = mandatory leading `HELLO` + map payload + `proto` negotiation +
     capabilities reply (BN-012); explicitly record the **shape ≠ auth-policy** principle (BN-012,
     BN-021) so no one reads "mandatory HELLO" as "mandatory auth."
   - **Error grammar** = the `both` superset; pin the exact spelling (bracketed auth codes vs bare
     tokens alongside `[CODE]`) — the one open spec decision from BN-011 — and include `NOPERM`
     in the recognized token set (Synap emits it today, BN-004).
   - **Caps** = 64 MiB configurable default; single in-flight default (BN-008).
   - **TLS** = one optional config/feature-gated rustls layer, off by default (BN-009).
   - **Push** = uniform client hook; `PUSH_ID` server→client-only; emission is a capability (BN-010).
2. Add corpus vectors: canonical-HELLO request/reply, canonical error `[CODE]` forms, plus
   **legacy-tolerance** vectors for every form being deprecated (no-HELLO connect, optional-HELLO,
   bare `NOAUTH`/`WRONGPASS`, 512 MiB cap config) — decode/accept-only, never emitted (mirrors
   WIRE-011/016 discipline).

**Gate GN0**: canonical-behavior spec merged; corpus carries canonical + legacy-tolerance vectors;
the shape/policy split is written down.

## 5.3 Phase N1 — Cheap wins (no cross-product coordination)

Depends on: a product being on Thunder (M2 per product) for the profile-flip form; the spec (GN0).
These three convergences need no dual-accept window — they are defaults or additive capabilities.

| Work | Action | Result |
|---|---|---|
| Caps | Make 64 MiB the family default; expose the config knob on Vectorizer + Synap servers (Nexus already has it); set Synap's profile to 64 with a documented override path | `max_frame_bytes`/`max_in_flight` become config-with-default, not per-product identity (BN-008) |
| TLS | Ensure the uniform optional `tokio-rustls` layer ships on `thunder::server`/`thunder::client` for all four (SRV-040/FR-29) — the family's *first running* RPC TLS, since Vectorizer's is spec-only (BN-007/BN-023; the old sequencing constraint is moot); flip profiles' `tls` to "config default" | Every product can be TLS-or-plaintext by config; behavioral surface identical (BN-009) |
| Push | Confirm every language's client ships the push hook (CLT-060); demote the profile `push` flag to a capability/documentation bit | Client push contract uniform; emission = capability (BN-010) |

**Gate GN1**: the `max_frame_bytes`, `max_in_flight`, `tls`, and `push` behavioral columns are
identical across all four profiles; the behavioral floor suite (Thunder T4.1/G4) passes under the
unified values.

## 5.4 Phase N2 — Error grammar convergence (server-first, per product)

Depends on: GN0; product on Thunder (so its client already parses the superset, CLT-050).

1. Migrate each product server's error-construction sites to emit the canonical superset
   `"[CODE] message"` (Nexus/Synap: bracket the RESP3 tokens or emit them as recognized bare codes
   per the GN0 spelling; Vectorizer: already bracket-code, minor alignment; Lexum: already `both`).
2. Clients need no change — Thunder's superset parser accepts both legacy and canonical by
   construction (BN-011). Keep legacy-error corpus vectors as decode-only.
3. Flip each product's `error_codes` profile field to the canonical value as its server migrates.

**Gate GN2**: every family server emits the canonical grammar; the `error_codes` column is identical;
legacy grammars survive only as decode-only tolerance vectors.

## 5.5 Phase N3 — Handshake convergence (the hard track, dual-accept, per product)

Depends on: GN0; products on Thunder (N3 changes `thunder::server` dual-accept once + profile flips).
This is the only phase with a per-product deprecation window. It is independent per product — a
product can sit at N3.a for as long as it wants without blocking any other.

- **N3.a — Servers dual-accept** (non-breaking). `thunder::server` gains a canonical accept path
  (leading HELLO → negotiate `proto`, reply with capabilities) alongside the profile's legacy path
  (Nexus: optional-HELLO + `AUTH` allowlist; Synap: no-HELLO immediate serve). The path is chosen by
  the first frame's command, so no ambiguity (BN-019). Add a HELLO-reply-side counter of legacy
  first-frames per product (telemetry for the eventual cut). Product-specific:
  - **Synap** puts the canonical HELLO in front of the auth it already has: HELLO credential fields
    feed the same shared-`UserManager` call its `AUTH` handler runs today; enforcement stays the
    `require_auth` toggle, so an open deployment accepts a credential-less HELLO and keeps serving
    un-credentialed connections (BN-017). The legacy no-HELLO(+`AUTH`) path stays dual-accepted, and
    the `SUBSCRIBE`/push wiring is untouched. No auth subsystem work exists on this path.
  - **Nexus** maps its `AUTH [api_key]`/`[user, pass]` semantics into HELLO map fields; its HELLO
    today is arg-less (registry errata — BN-023), so the canonical map *adds* `version`, identity
    and credentials rather than replacing a positional form (BN-016). Legacy optional-HELLO+AUTH
    still accepted.
  - **Vectorizer / Lexum** are already canonical — they are the reference; their work is verifying
    the shared `thunder::server` path matches their existing behavior (BN-015, BN-014).
- **N3.b — SDK flip** (non-breaking): each product's SDKs send the canonical leading HELLO — a
  Thunder profile-value flip to `hello_mandatory`/`map_payload`, since post-swap every SDK already
  runs `thunder::client` (CLT-002 branches). Old deployed clients keep using the legacy path against
  the dual-accepting server.
- **N3.c — Per-product legacy cut** (major): once a product's legacy-first-frame counter reads zero
  over its chosen window, remove that product's legacy accept path in a product major (WIRE-016
  discipline). No synchronized family major — each product cuts on its own evidence (BN-020, BN-021).

**Gate GN3**: every product's server dual-accepts and its SDKs default-emit canonical; the
`handshake`/`hello_style` columns are identical for every product still within its window; legacy
handshakes are demoted to decode/accept-only tolerance vectors.

## 5.6 Phase N4 — Retire the behavioral profile fields (finish line)

Depends on: GN1 + GN2 + GN3 for a given column.

1. For each behavioral column now identical across all registered profiles, **promote it out of the
   per-product profile into a single family constant** (a Thunder default) and drop the redundant
   per-product field in a minor (PRO-002 makes this non-breaking).
2. Update the conformance suite to assert **one** handshake behavior and **one** error behavior
   across all profiles (PRO-013), with the deprecated forms retained only as decode-only tolerance
   vectors.
3. The profile registry now carries only *identity* (`scheme`, `default_port`, the push-capability
   bit) and points at the family behavioral constant — the concrete realization of BN-013's
   "one canonical behavior with four address labels."

**Gate GN4**: the profile YAMLs' behavioral columns are collapsed to a single canonical set; "all
four speak exactly the same way" is a CI property (the corpus asserts one behavior; legacy is
tolerance-only); the only per-product deltas are scheme, port, command catalog, and deployment policy.

## 5.7 Effort and dependency summary

| Phase | Coordination | Calendar (rough, familiar engineer) | Blocks | Value shipped |
|---|---|---|---|---|
| N0 | spec decision | 2–4 days | N1–N4 | the single canonical definition + corpus |
| N1 | none (per product) | 3–5 days total | — | caps/TLS/push columns unified |
| N2 | none (per product, server-first) | 1 week total | — | one error grammar emitted family-wide |
| N3 | per-product window | weeks–months of *calendar* (mostly waiting on telemetry), days of *work* per product | — | one handshake shape family-wide |
| N4 | per column | 2–3 days per converged column | — | profile behavioral fields retired; CI asserts one behavior |

The *work* is small; the *calendar* is dominated by N3's evidence-gated deprecation windows, which
are deliberately slow and per-product. Crucially, N1 and N2 deliver "four of five dimensions
identical" within roughly two weeks of engineering once products are on Thunder — the visible
majority of "they speak the same way" arrives early and cheaply, and only the handshake trails.

## 5.8 Relationship to Thunder's roadmap

- **Prerequisite**: Thunder M2 (family Rust swap) for a product to participate via profile flips
  rather than legacy-listener edits (BN-022). N1 can start per product as that product hits M2.
- **N0** can be authored during Thunder M1 (the spec home and corpus already exist).
- **N3.c majors** are product releases, coordinated by each product on its own cadence — Thunder
  itself never needs a synchronized major (BN-020).
- **Push v-next** (Thunder T5.2) lands *once* into the now-uniform push contract instead of
  per-product — a downstream dividend of N1's push normalization (BN-010).

---

Back to the [index and executive summary](README.md).
