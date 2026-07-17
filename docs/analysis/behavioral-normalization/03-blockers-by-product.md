# §3 — Blockers by Product

> The four cheap dimensions (caps, TLS, push, errors) have essentially no per-product blocker — they
> converge by defaulting or additive shipping (§2). This section is about where the *handshake*
> migration actually bites, because that is where the four products are genuinely unequal. Ordered
> easiest → hardest adopter, which is the reverse of how much each has to change.
>
> Evidence base: `docs/analysis/01-current-state.md` §1.2 (the server-side handshake/push/cap/TLS
> table, with per-product file anchors), `Lexum/docs/analysis/hivellm-rpc/02-implementations.md`
> (F-011/F-012), the four `conformance/profiles/*.yaml`, and direct reads of the product RPC crates
> and listeners.

## 3.1 Lexum — zero blockers; the forcing function

### BN-014 — Lexum has no deployed RPC clients and is already specced to the canonical shape; if the canonical behavior is ratified before Lexum ships RPC, Lexum never builds a divergent handshake at all

- **Evidence**: Lexum has *no* binary RPC listener today — its `/umicp` endpoint is a bespoke
  HTTP-tunneled codec that shares no wire types with any client (`Lexum/.../04-lexum-adoption.md`
  F-019), and its family-RPC adoption is an unwritten reserved spec slot (SPEC-015, F-023). Its
  *planned* profile is already the canonical target: `handshake: hello_mandatory`,
  `hello_style: map_payload`, `error_codes: both` (`lexum.yaml:4-9`; adoption plan §4.3 — "Vectorizer's
  mandatory HELLO carrying `version` + `api_key` or JWT tenant token"). Thunder already plans to onboard
  Lexum green-field onto `Profile::lexum()` instead of a `lexum-protocol` crate (T2.5, T-019).
- **Impact**: Lexum is the cleanest possible adopter — there is nothing to migrate *off of*, because
  there is nothing there yet. This makes Lexum the **forcing function** for normalization exactly as
  it was for consolidation (T-019): if the canonical behavior (§2) is pinned before Lexum's RPC
  listener is written, Lexum implements the *one* family behavior from day zero and becomes living
  proof that a new product onboards onto a single canonical protocol with no transition and no
  profile divergence. If normalization is *not* pinned first, Lexum will implement the Vectorizer
  shape and become a fifth thing to reconcile later. **The cheapest moment to normalize Lexum is
  before it starts.**
- **Confidence**: high.

## 3.2 Vectorizer — near-zero; it *is* the reference handshake

### BN-015 — Vectorizer already implements the canonical handshake shape; its only convergence work is error-grammar spelling, and it is the TLS donor, not a blocker

- **Evidence**: Vectorizer's profile is the canonical target on the two hard sub-dimensions —
  `handshake: hello_mandatory`, `hello_style: map_payload` (`vectorizer.yaml:4-5`); the HELLO is the
  mandatory first frame carrying `version` + `token`/`api_key` + `client_name`
  (`dispatch.rs:3289-3311`), enforced by rejecting any other pre-auth first command
  (`dispatch.rs:189-192`), with a capabilities reply (`dispatch.rs:425-447`). On errors it emits
  `bracket_code` via `vectorizer_err_ctx` (`dispatch.rs:57-59`), which is the *base* of the
  canonical superset — alignment is spelling, not a model change (BN-011). On TLS it is the family's
  **design donor**: the optional-rustls posture is spec'd (`VECTORIZER_RPC.md:293-296`) but not
  wired (BN-007/BN-023) — an asset as a spec, not running code.
- **Impact**: Vectorizer is the reference implementation of the canonical handshake — the other three
  migrate *toward* what Vectorizer already does. Its own work is: (a) confirm the shared
  `thunder::server` handshake path reproduces its existing mandatory-HELLO behavior byte-for-byte (the
  corpus handshake vectors already pin the Vectorizer reply shape — SPEC-004 SRV-014, T-015); (b)
  minor error-string alignment to the pinned superset spelling. Its capabilities reply and spec'd
  TLS design become the family defaults. The once-feared ordering constraint (uniform TLS must land
  before Vectorizer's swap lest an rustls-enabled deployment regress) dissolves under evidence: its
  `RpcConfig` has no TLS keys, so no such deployment exists (BN-009).
- **Confidence**: high.

## 3.3 Nexus — moderate; fold `AUTH` into `HELLO` and make HELLO mandatory

### BN-016 — Nexus must move from optional-HELLO + separate-`AUTH` to mandatory-HELLO-with-credentials, and change its error prefixes; both are breaking for deployed Nexus clients and need the dual-accept window

- **Evidence**: Nexus is `handshake: auth_command`, `hello_style: positional_version`
  (`nexus.yaml:4-5`) — though the `hello_style` cell is an errata: the RPC `HELLO` is *optional* and
  **arg-less**, returning a metadata Map `{server, version, proto, id, authenticated}`
  (`rpc/dispatch/admin.rs:54-77`; the positional `[Int(1)]` is the RESP3 HELLO — BN-023).
  Authentication is a **separate** `AUTH` command accepting **either** `[api_key]` **or**
  `[username, password]` (`admin.rs:89-112`); a pre-auth allowlist (`PING/HELLO/AUTH/QUIT`,
  `dispatch/mod.rs:54`) gates the rest; errors use RESP3 prefixes `ERR/NOAUTH/WRONGPASS`
  (`mod.rs:84,104`, `admin.rs:97,107`; `NOPROTO` never fires on RPC — BN-004). The canonical shape
  (BN-012) requires: (1) the first frame is a HELLO **map**, not optional and not arg-less;
  (2) credentials ride *inside* that HELLO map rather than a later
  `AUTH` — and the map must be able to carry both the `api_key` form *and* the `[user, pass]` form
  Nexus supports; (3) errors move to the `[CODE]` superset.
- **Impact**: this is a genuine protocol change for Nexus, and it is breaking for any deployed Nexus
  client that (a) sends no HELLO (or today's arg-less one), or (b) authenticates via a standalone
  `AUTH` call. None of it is *hard code* — Thunder's server already constructs the unified HELLO reply
  covering the Nexus reply shape (`{server, version, proto, id, authenticated}`, SRV-014), and the
  HELLO map trivially carries `api_key`/`user`/`pass` fields. The cost is the **dual-accept window**
  (§4 BN-019): Nexus's `thunder::server` keeps the optional-HELLO + `AUTH`-command path alive while
  its SDKs switch to the canonical HELLO map, then cuts the legacy path in a Nexus major once
  telemetry shows no legacy first-frames. The username/password vs api_key duality is *not* a
  blocker — both are just fields in the HELLO map — but it must be written into the canonical spec so
  every product's HELLO map schema admits all three credential forms.
- **Confidence**: high.

## 3.4 Synap — the widest client gap: no HELLO at all (but auth already exists behind `require_auth`)

### BN-017 — Synap's RPC path already authenticates (`AUTH` + `NOAUTH` gate + `NOPERM` ACL behind `require_auth`, on the shared `UserManager`) but has **no HELLO** of any kind; the migration is "put a HELLO in front of the auth that exists," and the real cost is flipping six SDKs

- **Evidence**: the registry says `handshake: none`, `hello_style: null` (`synap.yaml:4-5`) — "v1
  legacy: no RPC-layer auth" — and an earlier read of this analysis repeated that, having found no
  `AUTH`/`HELLO` in the *dispatch* directory. The primary source contradicts it: Synap authenticates
  **inline in the RPC connection read loop**, not in dispatch. The gate is seeded from config
  (`let mut authenticated = !state.require_auth;`,
  `Synap/crates/synap-server/src/protocol/synap_rpc/server.rs:170`); `AUTH <password>` /
  `AUTH <user> <password>` calls the same shared `UserManager` the HTTP surface uses
  (`server.rs:203-228`); every other command answers `NOAUTH Authentication required.` until then
  (`server.rs:230-235`); an admin ACL answers `NOPERM` (`server.rs:239-249`). What Synap genuinely
  lacks is a **`HELLO` handler — nowhere on the RPC path** (BN-023 corrects the registry cell to
  `auth_command`). Synap is also the family's *only* push producer — `SUBSCRIBE` is dispatched on
  the RPC path (`dispatch/advanced.rs:719-746`) and the connection layer emits frames with
  `id = u32::MAX` (`server.rs:295-298`, wired at `:264-308`) — and the cap outlier (512 MiB,
  `synap-protocol/src/synap_rpc/codec.rs:21`, though its own SDKs already cap at 64 MiB —
  `sdks/rust/src/transport/mod.rs:119,182,221`; BN-006).
- **Why it is the hardest adopter anyway**: of the four products, Synap has no HELLO surface at all
  to build on — making the canonical HELLO mandatory changes the very first thing every Synap
  client does (today: a bare `AUTH`, or nothing when the deployment is open). And the flip is wide:
  six SDKs (analysis §1.3), of which several don't even agree on the `Request` byte shape today
  (map from Python/Go/Java, array from TS/Rust — the drift WIRE-013 already absorbs). Its
  dual-accept path must also leave the `SUBSCRIBE`/push wiring undisturbed.
- **Why it is more tractable than the registry suggests**: the expensive reading — "Synap must
  build RPC-layer authentication" — is simply **false**; the RPC loop already reaches the shared
  `UserManager`. The canonical HELLO's credential fields map 1:1 onto the existing
  `AUTH <password>` / `<user> <password>` semantics, so Synap's `thunder::server` HELLO handler
  *reuses* the verification call that exists today; enforcement stays the `require_auth` toggle (the
  shape ≠ policy split, BN-012 — which Synap itself already demonstrates in-family, exactly like
  Nexus's `auth_required`). An open deployment accepts a credential-less HELLO and keeps serving
  un-credentialed connections exactly as today. So the obligation is narrow: (a) accept a leading
  HELLO (with or without credentials) and reply with capabilities; (b) SDKs begin leading with it;
  (c) the legacy no-HELLO(+`AUTH`) path stays dual-accepted until telemetry is clean, then is cut in
  a Synap major.
- **Residual Synap-specific items** (all already handled elsewhere, noted so they are not mistaken
  for blockers): the 512 MiB cap converges to a 64 MiB default with an override, matching what its
  SDKs already enforce (BN-006/BN-008); its `SUBSCRIBE` push is an **asset** — the concrete push
  producer that *defines* the uniform push contract the other three adopt as a hook (BN-010); its
  map-shaped `Request` in the Py/Go/Java SDKs is a byte drift already absorbed by Thunder's decode
  tolerance (WIRE-013, T-005), orthogonal to behavioral normalization.
- **Impact**: Synap is still where the calendar cost of N3 concentrates — the widest first-frame
  gap and the most SDKs to flip — but no auth subsystem work exists anywhere on the critical path.
  The one thing N0 must still ratify explicitly is that *mandatory HELLO does not mandate
  credentials* (§5.2), so open Synap deployments stay open. And independent of normalization, the
  registry cell must be corrected (BN-023): as committed, a Thunder client on the `synap` profile
  cannot authenticate against a `require_auth` Synap at all.
- **Confidence**: high — anchored to primary source (`server.rs` read 2026-07-17); the precise size
  of the SDK-flip work is medium (six Synap SDKs vs the profile-flip ideal) and firms up once Synap
  is on Thunder (BN-022).

## 3.5 Difficulty ranking and the one cross-cutting dependency

| Product | Handshake gap to canonical | Error gap | Net difficulty | Role |
|---|---|---|---|---|
| **Lexum** | none (already specced canonical) | none (`both`) | **None** | forcing function — normalize before it ships |
| **Vectorizer** | none (already `hello_mandatory`+`map`) | spelling only | **Near-zero** | reference handshake; TLS donor |
| **Nexus** | fold `AUTH`→HELLO; optional→mandatory; arg-less→map | RESP3→superset spelling | **Moderate** | dual-accept window; credential-form schema decision |
| **Synap** | add a leading HELLO where none exists (auth already there, behind `require_auth`); flip 6 SDKs | RESP3(+`NOPERM`)→superset spelling | **Hardest** | HELLO in front of existing AUTH; defines the push contract |

- **The cross-cutting dependency** (BN-022, restated as a blocker): *none of these migrations are
  cheap until the product is on Thunder.* Before the swap, "Nexus adopts mandatory HELLO" means
  editing five hand-ported Nexus transports plus the server; after the swap it means a
  `thunder::server` dual-accept path plus a profile flip. Every per-product blocker above assumes the
  product has completed Thunder M2/M3 — which is why normalization is sequenced as Thunder's sequel,
  not its sibling.
- **Confidence**: high.

---

Next: [§4 — Migration path](04-migration-path.md).
