# §2 — Feasibility by Dimension

> For each of the five behavioral dimensions: the single **canonical behavior** to converge on, why
> it subsumes the others, and the cost of eliminating the divergence. Ordered cheapest → hardest, so
> the reader sees the four near-free wins before the one genuinely hard problem.
>
> The recurring test for "can this be eliminated" is not "is one behavior technically superior" — all
> five already have a defensible best — but **"can all four products emit/obey the one behavior
> without breaking a deployed client, and if not, is there a bounded transition that gets there."**

## 2.1 Caps and in-flight bound — normalize by defaulting (feasible now)

### BN-008 — Converge on 64 MiB default + operator-configurable everywhere; Synap keeps 512 as an override until it decides otherwise. Zero interop cost.

- **Canonical behavior**: `max_frame_bytes` default **64 MiB**, validated before allocation, tunable
  per deployment (the Nexus posture — `rpc.max_frame_bytes` + env). `max_in_flight` a single
  configurable default (256 is the family plurality; Nexus's 1024 becomes a config value, not a
  profile identity).
- **Why it subsumes the others**: Thunder already mandates exactly this (SPEC-001 WIRE-020:
  "configurable per connection/profile with default 64 MiB"). The only outlier, Synap at 512 MiB
  (BN-006), is already effectively 64 MiB in practice because its **own SDK hardcodes 64** (T-005) —
  so no real Synap frame today exceeds 64 MiB, and lowering the *server's* default to 64 refuses
  nothing that currently succeeds. An operator with a genuine >64 MiB Synap path sets one config
  value; everyone else inherits the family default.
- **Cost**: trivial. It is a default-value change plus exposing the config knob on the two products
  (Vectorizer, Synap) that hardcode it. No client changes, no wire change, no transition window —
  the cap is checked identically regardless of its value.
- **Verdict**: **eliminable immediately.** The `max_frame_bytes`/`max_in_flight` profile fields
  collapse to "config with a family default," which is not a per-product behavioral divergence at
  all.
- **Confidence**: high.

## 2.2 TLS — normalize by shipping the one implementation to all four (feasible, additive)

### BN-009 — Ship Thunder's single optional `tokio-rustls` layer (server + client) uniformly; each deployment opts in. Nobody has a conflicting TLS behavior to migrate off of.

- **Canonical behavior**: one optional, config-gated TLS layer — server `SRV-040`
  (`tls.cert_path`/`key_path`, feature-gated, no STARTTLS) and client `FR-29` (rustls/native) — 
  available to every profile, **off by default**, with the family's loopback-bind guidance as the
  documented posture when it is off.
- **Why it subsumes the others**: TLS is the only dimension where the divergence is a *missing
  capability*, not a *conflicting behavior* — and the primary-source sweep sharpened this: **no
  product runs RPC TLS today** (BN-007). Vectorizer contributes the config-gated rustls *design*
  (spec'd, never wired — BN-023); nobody ships it. Making it uniform is *adding* one capability to
  all four via code Thunder already owns — not reconciling four different TLS stacks, and not even
  preserving one existing stack.
- **Cost**: low, and *additive-only*. An off-by-default encrypted-transport option cannot break a
  deployed plaintext client — the client keeps connecting in plaintext until an operator turns TLS on
  at both ends. The `tls` profile field stops meaning "which product am I" and starts meaning "did
  this operator enable it," i.e. it becomes deployment config exactly like caps.
- **One sequencing constraint, now dissolved** (inherited from SPEC-004 SRV-040): the worry was
  that a Vectorizer deployment already running rustls would regress if the uniform layer landed
  after its Thunder swap. The source shows `RpcConfig` has no TLS keys at all
  (`vectorizer.rs:117-131`, BN-007/BN-023), so no deployment *can* have RPC TLS on — the constraint
  is moot. Keep it only as a one-line re-verify at the start of the work.
- **Verdict**: **eliminable, additively.** The behavioral surface becomes identical (all four can be
  TLS or plaintext by config); the only residual is a per-deployment on/off, which is not a protocol
  divergence.
- **Confidence**: high.

## 2.3 Push — normalize the client contract; demote the divergence to a capability (feasible)

### BN-010 — The wire reservation is already uniform; make the client push-hook floor uniform and "enabled vs reserved" reduces to a per-product capability, not a dialect.

- **Canonical behavior**: one rule for every product — *`PUSH_ID` is a server→client-only channel:
  clients never send it as a request id, servers never accept it as one, and every client routes an
  inbound `PUSH_ID` frame to a registered push hook* (SPEC-003 CLT-060). Whether a given server
  *ever emits* a push frame is then a property of its command catalog (Synap's `SUBSCRIBE` does; no
  other product has a command that does — yet).
- **Why it subsumes the others**: this dimension is already 90% normalized at the wire (BN-005). All
  four profiles reserve `PUSH_ID` identically; Thunder's client contract already requires every
  client to handle an inbound push frame. The `push: enabled|reserved` flag today governs two things
  that pull apart cleanly:
  1. **Client routing** — already uniform under CLT-060 (route `PUSH_ID` to the hook). No divergence
     to remove.
  2. **Server refusal of a client-sent `PUSH_ID`** — SPEC-004 SRV-013 already makes *every* server
     refuse it. No divergence to remove.
  3. **Server *emission*** — the only real difference, and it is a capability (Synap has a subscribe
     command; others don't), identical in kind to "Nexus has `CYPHER`."
- **Cost**: low. The one nuance worth resolving explicitly: under a strict reading, a `reserved`
  client that receives an unexpected `PUSH_ID` frame *poisons the connection* (CLT-060 →
  CLT-014), whereas an `enabled` client delivers it to the hook. Normalizing means **every** client
  ships the hook (harmless when unused) and *optionally* treats an unexpected push as a poison — but
  the hook's presence is the same everywhere. This makes a future family-wide push feature (task
  completion, streaming) land once, not per-product (Thunder T5.2 already anticipates this).
- **Verdict**: **eliminable.** After normalization the profile need not carry a `push` behavioral
  flag at all — it carries at most a "this product ships a push-producing command" documentation bit.
- **Confidence**: high.

## 2.4 Error grammar — normalize by emitting one canonical superset (feasible, small blast radius)

### BN-011 — Every server emits the `both` superset (`"[CODE] message"` with `NOAUTH`/`WRONGPASS` as recognized codes); every client parses that one grammar. Lexum already targets it.

- **Canonical behavior**: one grammar — the bracket-code form `"[<CODE>] <message>"` where the RESP3
  auth tokens (`NOAUTH`, `WRONGPASS`, `NOPERM`, `NOPROTO`) are recognized `CODE` values, so a single
  parser yields `{code, class, message}` for every server. (`NOPERM` joins the set per BN-004 — Synap
  emits it today and no spec models it; `NOPROTO` is carried for the convention even though no RPC
  path emits it yet.) This is precisely Lexum's `error_codes: both` (`lexum.yaml:9`) and the union
  F-011's error row recommended ("auth prefixes from Nexus + SPEC-003 code prefix from Vectorizer").
- **Why it subsumes the others**: the superset is strictly more expressive than either base grammar,
  and — decisively — **no deployed client parses either grammar today** (BN-004; ARCHITECTURE.md §3.2
  "donor: none"). Because the string is currently opaque to every SDK, changing what servers *emit*
  has essentially no client-code blast radius: the only observers of the specific prefix today are
  humans reading logs. Thunder's client already parses the superset (CLT-050/051, PRO-014), so the
  *client* side is normalized the moment products swap onto Thunder; the remaining work is
  **server-side string emission**.
- **Cost**: medium — it touches each server's error-construction sites (Nexus emits `WRONGPASS …`
  today; it would emit `[WRONGPASS] …` or keep `WRONGPASS` as a recognized bare code — a decision to
  pin in the canonical spec). But it is server-local, mechanical, and guarded: Thunder's superset
  parser accepts *both* the legacy bare-prefix form and the bracketed form during the transition (the
  `both` convention is definitionally a superset), so a client never breaks while servers migrate
  their strings one at a time.
- **Verdict**: **eliminable over a short server-side window.** The `error_codes` field collapses to a
  single value (`both`, renamed "canonical") once every server emits it; the two base conventions
  become decode-only legacy.
- **Confidence**: high (medium only on the exact canonical spelling — bracketed auth codes vs bare
  auth tokens alongside bracket codes — which is a spec decision, not a feasibility question).

## 2.5 Handshake — the crux: one canonical HELLO, with auth *policy* separated from handshake *shape*

### BN-012 — Converge on mandatory-`HELLO` + map payload + `proto` negotiation as the handshake **shape**, and keep auth **enforcement** a per-deployment policy. This is what makes the hardest dimension tractable.

- **Canonical behavior**: the first frame on every RPC connection is a `HELLO` carrying a Map
  `{version, client_name, and optionally token | api_key | [user, pass]}`; the reply is a Map
  carrying `{protocol_version/proto, server, capabilities, authenticated}`. This is the
  Vectorizer/Lexum `hello_mandatory` + `map_payload` shape (BN-003), chosen because it is the only
  model that is simultaneously:
  1. **already the plurality** (2 of 4 profiles; and Lexum is green-field, so new products default
     to it — §3 BN-017);
  2. **the richest** — it carries version negotiation, client identity, credentials, *and* a
     capabilities reply in one round trip, which the other two models cannot (Nexus splits HELLO and
     AUTH; Synap has AUTH but no HELLO at all — BN-003/BN-023);
  3. **a superset that can express the others' semantics**: Nexus's `AUTH [api_key]` or
     `[user, pass]` and Synap's `AUTH <password>` / `<user> <password>` all fit as credential fields
     in the HELLO map, feeding the same server-side verification each product already runs; Nexus's
     arg-less HELLO gains the map's `version` (its `proto` already rides the reply); an open
     deployment's "no credentials" is a HELLO map that simply omits the credential fields.
- **The decisive reframing — shape ≠ policy**: "mandatory HELLO" is a statement about **frame
  ordering and negotiation**, *not* about whether authentication is required. A server may require
  the first frame to be `HELLO` while enforcing **no** credentials — and the split is not
  hypothetical: it is already how both credentialed-RPC products behave, each gating enforcement on
  its own config toggle (Nexus `auth_required`, `rpc-wire-format.md:268`; Synap `require_auth`,
  `synap_rpc/server.rs:170`). This split is what shrinks the Synap work from the feared "build RPC
  authentication" (a misreading the registry's `none` invites — Synap's RPC loop already reaches the
  shared `UserManager`, BN-023) to "put a leading HELLO in front of the auth that already exists"
  (§3 BN-017). Auth *enforcement* (does the server reject a HELLO with no/invalid credentials?)
  legitimately remains a per-deployment policy, exactly like TLS on/off — it is **not** a protocol
  dialect and does not need to be normalized to make the four "speak the same way." What normalizes
  is the *shape*: everyone leads with HELLO, negotiates `proto`, and receives a capabilities reply.
- **Why it is still the hard one**: even reduced to shape, converging changes *the first frame a
  correct client sends* for two of four products:
  - **Nexus** clients that authenticate via a separate `AUTH` command, and treat `HELLO` as optional,
    must move to sending a leading HELLO map (§3 BN-016).
  - **Synap** clients that send *nothing* first (or a bare `AUTH`) must begin sending a leading
    HELLO (§3 BN-017).
  Both are breaking changes for *already-deployed* clients, so unlike the other four dimensions this
  one cannot be a silent default flip — it needs a negotiated dual-accept transition (§4).
- **Cost**: high relative to the others, low in absolute terms. The server change is "accept a
  leading HELLO frame and populate the reply"; Thunder's server already builds the unified reply
  (SPEC-004 SRV-014 explicitly covers *both* family reply shapes). The expensive part is not code —
  it is the **coordination**: every product's servers must dual-accept before its clients switch, and
  the legacy path (no-HELLO for Synap; optional-HELLO+AUTH for Nexus) must live through a deprecation
  window (§4 BN-019/BN-020).
- **Verdict**: **eliminable, but only via a bounded, negotiated migration** — not a defaulting
  exercise. The `handshake` and `hello_style` fields are the *last* profile rows to collapse, and the
  whole "make them speak exactly the same" question effectively reduces to "will the family run this
  one migration."
- **Confidence**: high on the target and the shape/policy split; medium on the calendar cost, which
  depends on product release cadences (§4 risk register).

## 2.6 The consolidated verdict

### BN-013 — Full behavioral normalization is feasible; the four-row profile collapses to (essentially) one, and 4 of 5 dimensions get there at near-zero cost. The entire residual difficulty is one coordinated handshake migration.

| Dimension | Canonical behavior | Eliminable? | Cost | Gating constraint |
|---|---|---|---|---|
| Caps + in-flight | 64 MiB default, configurable; single in-flight default | **Yes, now** | Trivial | none (Synap SDK already at 64) |
| TLS | one optional rustls layer, off by default, all four | **Yes, additively** | Low | none — no deploy can have RPC TLS on (BN-007/BN-023) |
| Push | uniform client hook; emission = capability | **Yes** | Low | none |
| Error grammar | `both` superset emitted by every server | **Yes, over a short window** | Medium (server strings) | superset parser accepts legacy during transition |
| Handshake | mandatory HELLO + map + proto; auth = policy | **Yes, via migration** | High (coordination) | dual-accept per product before clients switch |

- **Reading**: after this work the profile registry does **not** vanish — it converges. The
  behavioral fields (`push`, `max_frame_bytes`, `error_codes`, `tls`, and eventually `handshake`/
  `hello_style`) all reach a single family value; what remains per-product is **identity**
  (`scheme`, `default_port`, command catalog) and **deployment policy** (auth required? TLS on? cap
  override?) — neither of which is a "they speak differently" problem (BN-001). Concretely, the
  target end state is a profile table whose behavioral columns are all identical, i.e. a single
  canonical behavior with four address labels on it.
- **The one honest caveat**: "essentially one row" is exact for four dimensions and *shape-exact* for
  the handshake. Auth-*enforcement* policy (an open deployment vs a credentialed one — both Synap and
  Nexus already expose exactly this as a config toggle) stays a deployment choice by design — which
  is correct: normalizing *whether a deployment requires a password* was never the goal, and forcing
  it would be a security regression for open deployments or an unwanted mandate for the others. "Exactly the same way" is achieved at the
  protocol-behavior layer; auth policy is deliberately left where it belongs.
- **Confidence**: high.

---

Next: [§3 — Blockers by product](03-blockers-by-product.md) — where the handshake migration actually
bites, product by product.
