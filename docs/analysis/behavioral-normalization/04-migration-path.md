# §4 — Migration Path: Converging Without a Flag Day

> The four cheap dimensions (caps, TLS, push, errors) converge by defaulting or by additive shipping
> — no coordination beyond a spec decision. This section is about the one dimension that changes a
> deployed client's first frame — the handshake — and how to eliminate it without a flag day. The
> family has already run exactly this kind of migration once (Synap's `Bytes` canonicalization), so
> the template is not hypothetical.

## 4.1 The negotiation channel already exists

### BN-018 — `HELLO`/`proto` is the built-in version-negotiation mechanism; behavioral convergence rides it rather than inventing a new one

- **Evidence**: the wire already carries an explicit protocol integer — `HELLO.proto = 1`
  (`Nexus/docs/specs/rpc-wire-format.md:287-291`), and Thunder freezes it as the sole vehicle for any
  future negotiated change: "a hypothetical v2 is a new negotiated `proto` integer" (SPEC-001
  WIRE-004; PRD NFR-01). The HELLO reply is server-constructed and already designed to carry
  `capabilities` (SPEC-004 SRV-014). Removing a decode tolerance is defined as a **major** event that
  requires every family server to have stopped emitting the legacy form first (SPEC-001 WIRE-016).
- **Impact**: the family already has (a) a place to advertise what a server supports (the HELLO
  reply / capabilities), (b) a version integer to gate behavior on, and (c) a written rule that
  legacy forms are accepted until a coordinated major. Normalization does not need a new protocol
  mechanism — it needs to *use* these three for handshake and error convergence the same way Thunder
  already uses them for the `Bytes`/`Request`-shape byte drifts. The canonical handshake is
  advertised in the HELLO reply's capabilities; the `proto` integer distinguishes a legacy client
  from a canonical one where the first-frame shape alone is ambiguous.
- **Confidence**: high.

## 4.2 Dual-accept: every server tolerates legacy and canonical during the window

### BN-019 — Each convergence is server-first and dual-accepting; the family's `Bytes` migration is the working template

- **The template** (already sanctioned, PRD NFR-04): Thunder's *one* pre-approved behavioral wire
  change is Synap's `Bytes` int-array → bin canonicalization, staged **server-first** — the server
  starts emitting the canonical form while Thunder decodes the legacy form "forever until a major"
  (analysis §4 risk register; T-005; WIRE-011). Behavioral normalization generalizes this exact
  pattern to the handshake and the error grammar:

  | Dimension | Legacy form accepted during window | Canonical form emitted | Keyed off |
  |---|---|---|---|
  | Handshake | no-HELLO, bare `AUTH` (Synap) / optional-HELLO+`AUTH` (Nexus) | mandatory leading HELLO map | first-frame command + `proto`; server dual-accepts |
  | Error grammar | bare `NOAUTH`/`WRONGPASS`/`ERR` prefixes | `[CODE] message` superset | superset parser (accepts both by construction, BN-011) |
  | Caps | 512 MiB (Synap) | 64 MiB default | pure config; no wire ambiguity, no window needed |
  | TLS | plaintext | plaintext-or-TLS by config | additive; no window needed |

- **How handshake dual-accept works concretely**: a normalized server, on a fresh connection, accepts
  **either** a leading `HELLO` (canonical path — negotiate, reply with capabilities) **or** a
  non-HELLO first frame (legacy path — for Nexus, apply the old pre-auth allowlist; for Synap, apply
  its existing `AUTH`/`NOAUTH` gate exactly as today). Because the two paths are distinguished by
  the first frame's command string,
  no ambiguity exists — a server never has to guess. The server advertises canonical support in its
  HELLO reply so a canonical client knows it is talking to a migrated server; a legacy client that
  never sends HELLO simply never learns, and keeps working.
- **Sequencing** (per product, inherited from Thunder's swap ordering T2.x): (1) server dual-accepts
  canonical + legacy; (2) the product's SDKs switch to sending canonical (a Thunder profile-value
  flip, since post-swap every SDK already runs Thunder's client); (3) after a deprecation window with
  telemetry showing no legacy first-frames, the legacy accept path is removed in a **major**
  (WIRE-016 rule). Steps 1–2 are non-breaking; only step 3 is a major, and only after evidence.
- **Impact**: no product ever has a flag day. A deployed old client keeps working against a migrated
  server for the entire window; a new client works against both. The cost is carrying two accept
  paths in the server for one deprecation cycle — bounded, and already the family's accepted price
  for the `Bytes` change.
- **Confidence**: high.

## 4.3 The profile becomes a convergence ledger, then sheds its behavioral fields

### BN-020 — Normalization does not delete the profile registry; it drives every product's behavioral row to a single canonical row, then retires the fields that reached parity

- **Mechanics**: the profile registry (`conformance/profiles/*.yaml`, SPEC-002 PRO-010) is the
  natural bookkeeping for the migration. Today the behavioral columns differ per product (BN-001
  table). The convergence proceeds column by column:
  1. **Caps / in-flight / TLS** flip to the family default the moment the cheap-win work lands (§2);
     the fields become "config default," documented once, not per product.
  2. **Error grammar** flips to `both`/canonical per product as each server's string emission
     migrates; the two base values become decode-only legacy the client always tolerates.
  3. **Handshake / hello_style** flip to `hello_mandatory`/`map_payload` per product as each
     completes its dual-accept window; until then, the profile row *records which stage a product is
     in* — it is a live migration ledger, not a permanent fork.
  4. When a behavioral column is identical across all registered profiles, it is **promoted out of
     the per-product profile into a single family constant** (a Thunder default), and the per-product
     field is dropped in a minor (adding a field with a default is minor; removing a now-redundant
     one that every product set to the same value is likewise non-breaking — PRO-002).
- **End state**: the profile shrinks to what BN-001 identified as legitimately per-product —
  `scheme`, `default_port`, the "does this product ship a push-producing command" bit, and the
  product's command catalog (which was never in the profile). The behavioral surface is one canonical
  behavior; the profile is a table of *addresses*, not *dialects*. This is the precise sense in which
  "they all speak exactly the same way" is achieved: the registry that today encodes their
  differences ends up encoding only their names.
- **Impact**: the migration has a visible, testable finish line — the conformance suite (which
  exercises every registered profile, PRO-013) goes from asserting *four* handshake/error behaviors
  to asserting *one*, with the legacy paths demoted to decode-only tolerance vectors. Progress is not
  a vibe; it is the diff of the profile YAMLs over time.
- **Confidence**: high.

## 4.4 Backward-compatibility guarantees and what stays deliberately un-normalized

### BN-021 — The migration honors Thunder's non-breaking-adoption rule; auth policy, scheme, port and commands are intentionally left per-product

- **Guarantees kept**:
  - **Public SDK APIs never change** (PRD NFR-04). Normalization changes *transport behavior*
    (what the first frame is, what an error string looks like), not the product SDK's surface. A
    Nexus SDK user still calls the same typed method; underneath, the transport now leads with a
    canonical HELLO.
  - **Registry versions only, never git paths** (NFR-06) — products adopt each convergence by taking
    a released Thunder version and flipping a profile value, so a product upgrades on its own cadence.
  - **Legacy accepted until a coordinated major** (WIRE-016) — no deployed client is stranded within
    a major series.
- **Deliberately NOT normalized** (restating BN-001/BN-012 as guarantees, so nobody over-reaches):
  - **Auth *enforcement* policy** — whether a deployment requires credentials — stays per-deployment
    config. Forcing Synap-open deployments to require passwords would be a behavioral mandate, not a
    protocol normalization; forcing the credentialed products to accept anonymous connections would
    be a security regression. The *handshake shape* normalizes; the *auth requirement* does not, by
    design.
  - **Scheme, default port, command catalog** — product identity, never touched (BN-001).
- **Risk register**:

  | Risk | Mitigation |
  |---|---|
  | Canonical HELLO credentials must round-trip into each product's existing verification (the earlier "Synap has no RPC auth path to reuse" reading was refuted by primary source — BN-017/BN-023) | HELLO map credential fields feed the same calls the products run today (Nexus `AUTH` handler, Synap's shared `UserManager`); enforcement stays each deployment's `require_auth`/`auth_required` toggle; an open deployment sends a credential-less HELLO (BN-012, BN-017) |
  | A deployed client is missed and its legacy first-frame breaks after the major | The major only lands after telemetry (HELLO-reply-side counters of legacy first-frames) shows zero legacy traffic per product; the window is evidence-gated, not calendar-gated |
  | Error-string change breaks a user grepping logs | Strings are advisory; the structured `code`/`class` is the contract (SPEC-003 CLT-052 already forbids branching on message text). Communicate in release notes; the superset keeps the old tokens visible as codes |
  | Products refuse to run a coordination they see no local benefit from | The four cheap dimensions ship value with no coordination at all; the handshake migration is opt-in per product and can trail indefinitely without blocking the others (the profile ledger tolerates mixed stages) |
  | Coordinating a family-wide major is heavy | Only the *legacy-path removal* is a major, and it can be per-product (each product cuts its own legacy accept when its own telemetry is clean) — there is no single synchronized family major |

- **Confidence**: high.

---

Next: [§5 — Execution plan](05-execution-plan.md) — the phased, gated path that layers this onto
Thunder's existing milestones.
