# §1 — The Behavioral Divergence Inventory

> Evidence base: the four family profiles as committed data
> (`conformance/profiles/{synap,nexus,vectorizer,lexum}.yaml`), the profile model
> (`docs/specs/SPEC-002-profiles.md`), the wire binding (`docs/specs/SPEC-001-wire-format.md`), the
> canonical wire spec (`docs/spec/rpc-wire-format.md` / `Nexus/docs/specs/rpc-wire-format.md`), and
> the two prior studies this one builds on: Thunder's own feasibility analysis
> (`docs/analysis/`, findings T-001..T-030) and the Lexum divergence table
> (`Lexum/docs/analysis/hivellm-rpc/02-implementations.md`, F-011/F-012).
>
> Findings in this analysis are numbered **BN-001..BN-NNN** (Behavioral Normalization), a distinct
> prefix from the sibling `T-` (Thunder feasibility) and `F-` (Lexum) series so cross-references
> never collide.

## 1.1 What "speak exactly the same way" means — and what it does not

The question this analysis answers is narrower and sharper than the one Thunder's feasibility study
already settled. That study proved the family can share **one codec and one client contract** and
absorb the products' differences as a declarative **Profile** — its explicit goal is *"Profiles,
not forks"* (PRD §3 Goal 5). A profile is the measure of *residual behavioral divergence that
Thunder chose to parameterize rather than remove*. The four registered profiles are four different
rows:

| Dimension | synap | nexus | vectorizer | lexum |
|---|---|---|---|---|
| `handshake` | **none** | **auth_command** | **hello_mandatory** | hello_mandatory |
| `hello_style` | — (null) | **positional_version** | **map_payload** | map_payload |
| `push` | **enabled** | reserved | reserved | reserved |
| `max_frame_bytes` | **512 MiB** | 64 MiB | 64 MiB | 64 MiB |
| `max_in_flight` | 256 | **1024** | 256 | 256 |
| `error_codes` | **resp3_prefixes** | resp3_prefixes | **bracket_code** | **both** |
| `tls` | off | off | **optional_rustls** | reserved_config |
| *scheme / port* | `synap://` 15501 | `nexus://` 15475 | `vectorizer://` 15503 | `lexum://` 17001 |

*(Source: the four `conformance/profiles/*.yaml` files verbatim. Three of these cells are now
known to mischaracterize the products they describe — `synap.handshake`, `nexus.hello_style`,
`vectorizer.tls` — see the registry errata in BN-023, §1.7.)*

**Normalization = collapsing the behavioral rows of this table to one.** A client (or an operator,
or a runbook) connecting to any family server should observe an identical handshake, identical
error grammar, identical push contract, identical cap semantics, and an identical TLS posture — so
that the only thing that distinguishes "talking to Nexus" from "talking to Synap" is the **address
and the vocabulary**, never the **protocol behavior**.

### BN-001 — Two of the seven profile fields are *identity*, not *divergence*; normalization targets the other five (plus one tuning knob)

- **Evidence**: `scheme` and `default_port` (`nexus.yaml:2-3`, etc.) are what make a `nexus://…:15475`
  endpoint *be Nexus*; the command catalog (Nexus `CYPHER`/`KNN_SEARCH`, Vectorizer `search.basic`,
  Synap KV verbs, Lexum `search.query` — `Lexum/docs/analysis/hivellm-rpc/04-lexum-adoption.md`
  F-024) is the product's public vocabulary. Thunder already rules these permanently product-owned
  (ARCHITECTURE.md §7; SPEC-002 PRO-012 makes scheme/port *data* but keeps them per-product).
- **Impact**: "Exactly the same way" is a statement about the **five behavioral dimensions**
  (`handshake`, `push`, `max_frame_bytes`, `error_codes`, `tls`) plus the `max_in_flight` tuning
  knob and the `hello_style` sub-field of handshake — **not** about scheme, port, or commands.
  Conflating the two would demand Synap answer `CYPHER` or Nexus listen on 15503, which is not the
  goal and never will be. Keeping identity and behavior separate is what makes the target
  achievable: it shrinks the problem from "make four products the same product" to "make four
  products obey the same protocol rules."
- **Confidence**: high.

### BN-002 — The wire *layer* is already normalized; every remaining divergence is above it, in the connection's behavior

- **Evidence**: framing (`u32 LE + MessagePack body`), the 8-variant value model, the
  externally-tagged encoding, and the `PUSH_ID = u32::MAX` reservation are byte-identical across all
  three implementations by construction — `nexus-protocol` "matches SynapValue byte-for-byte"
  (`Nexus/docs/specs/rpc-wire-format.md:63-65`), `vectorizer-protocol` is "ported byte-for-byte from
  Synap" (analysis T-001), and SPEC-001 WIRE-001..005 binds all four Thunder targets to the same
  bytes. Even the two live byte-drifts (int-array `Bytes`, map-shaped `Request`) are already being
  driven to a single canonical form with decode tolerances (WIRE-010..013, T-005).
- **Impact**: This is the good news that sizes the whole effort. Nobody has to renegotiate *bytes*;
  the frame a client puts on the wire is already the same everywhere. What differs is what happens
  **around** those frames — who must authenticate and how, what an error string looks like, whether
  a server ever pushes, how big a frame may be, and whether the socket is encrypted. Normalization
  is therefore a **behavioral** project layered on Thunder's **structural** one, not a second wire
  fork. Every dimension below lives in `thunder-client` + `thunder-server` behavior and the profile
  that drives them, never in `thunder-wire`.
- **Confidence**: high.

## 1.2 Dimension 1 — Handshake (the widest gap)

### BN-003 — Three mutually incompatible handshake models coexist, and two products cannot even agree on the shape of HELLO

- **Evidence**: the `handshake` field takes three distinct values across four products
  (`synap.yaml:4` `none`; `nexus.yaml:4` `auth_command`; `vectorizer.yaml:4` and `lexum.yaml:4`
  `hello_mandatory`), formalized in SPEC-002 PRO-001:
  - **Synap `none` (registry) — in truth: `AUTH`-command, without HELLO.** The registry cell and
    the prior study's "auth is HTTP-only" reading (`01-current-state.md:13`) are contradicted by the
    source: the RPC listener authenticates **inline in its read loop** — the gate is seeded from
    config (`authenticated = !state.require_auth`,
    `Synap/crates/synap-server/src/protocol/synap_rpc/server.rs:170`), an `AUTH <password>` /
    `AUTH <user> <password>` handler calls the same shared `UserManager` HTTP uses
    (`server.rs:203-228`), every other command is refused with `NOAUTH` until authenticated
    (`server.rs:230-235`), and an admin ACL answers `NOPERM` (`server.rs:239-249`). With
    `require_auth` off — the posture the registry's `none` actually describes — the connection is
    usable immediately with zero handshake. What Synap genuinely lacks is a **`HELLO` handler**:
    there is none anywhere on the RPC path (BN-023).
  - **Nexus `auth_command`** — `HELLO` is *optional*; authentication is a *separate* `AUTH` command
    accepting **either** `[api_key]` **or** `[username, password]`
    (`Nexus/crates/nexus-server/src/protocol/rpc/dispatch/admin.rs:89-112`); a pre-auth allowlist
    (`PING/HELLO/AUTH/QUIT`, `dispatch/mod.rs:54`) gates everything else
    (`Nexus/docs/specs/rpc-wire-format.md:118-126`).
  - **Vectorizer/Lexum `hello_mandatory`** — the **first frame MUST be `HELLO`**, carrying
    credentials inline in a Map, and the reply carries `capabilities` — enforced at
    `Vectorizer/crates/vectorizer-server/src/protocol/rpc/dispatch.rs:189-192` ("authentication
    required: send HELLO first"; `vectorizer.yaml:4-5`).
  - On top of that, `hello_style` splits the two products that *do* use HELLO structurally — but
    not the way the registry says. `nexus.yaml:5` claims **positional** `[Int(1)]`; that is a
    RESP3-ism. The RPC `HELLO` takes **no arguments** and returns a metadata Map
    `{server, version, proto, id, authenticated}` (`admin.rs:54-77`; spec row `HELLO | [] | Map`,
    `rpc-wire-format.md:117`) — a registry errata (BN-023). Vectorizer sends a **Map**
    `{version, token|api_key, client_name}` (`dispatch.rs:3289-3311`).
- **Impact**: this is the single widest divergence and the one that actually changes what bytes a
  correct client must send first. A client cannot be handshake-agnostic today: against Synap it must
  send `AUTH` (or nothing, when the deployment is open), against Nexus it may send `AUTH`, against
  Vectorizer it must lead with a HELLO map or be rejected. The corrected inventory narrows the gap,
  though: Synap and Nexus share the *same* auth model (an `AUTH` command behind a per-deployment
  `require_auth`/`auth_required` toggle, against a shared user store) — they differ only in whether
  an optional metadata `HELLO` exists. The real three-way split is **no-HELLO+AUTH (Synap) /
  optional-arg-less-HELLO+AUTH (Nexus) / mandatory-HELLO-map (Vectorizer, Lexum)**. Thunder's client
  already encodes these as `CLT-002` branches — normalization means choosing **one** to be the
  family behavior and migrating the others onto it. Because the models disagree on *the first
  frame*, this is the dimension whose elimination is a real protocol change, not a defaulting
  exercise (feasibility in §2 BN-012; blockers in §3).
- **Confidence**: high.

## 1.3 Dimension 2 — Error grammar

### BN-004 — Two error-string grammars are in use, Lexum is specced to straddle both, and no deployed client parses either

- **Evidence**: `error_codes` takes two base values plus a union: Nexus and Synap emit
  **`resp3_prefixes`** (`ERR`/`NOAUTH`/`WRONGPASS`/`NOPROTO`,
  `Nexus/docs/specs/rpc-wire-format.md:103-106`); Vectorizer emits **`bracket_code`**
  (`"[<code>] message"`, built by `vectorizer_err_ctx` at
  `vectorizer-server/src/protocol/rpc/dispatch.rs:57-59`, codes from `VectorizerError::code()`,
  `vectorizer-core/src/error/kind.rs:106-144`); Lexum's profile is **`both`** — deliberately a
  superset parser (`lexum.yaml:9`, "bracket_code (SPEC-003 codes) + resp3 auth prefixes"). SPEC-001
  WIRE-040 freezes the fact that *Thunder never invents a third convention* — it models exactly
  these two. The primary-source sweep pins the actual RPC emission sites and adds two nuances the
  registry misses: **(a)** Synap emits a third auth-family token, **`NOPERM this command requires
  admin privileges`** (`synap_rpc/server.rs:243-245`), alongside `NOAUTH` (`server.rs:232`) and
  `WRONGPASS` (`server.rs:222-224`); Nexus emits `NOAUTH` (`rpc/dispatch/mod.rs:84`), `WRONGPASS`
  (`rpc/dispatch/admin.rs:97,107`) and `ERR` (`mod.rs:104`). **(b)** `NOPROTO` is convention-only on
  the RPC path — its lone construction site is the RESP3 surface (`resp3/command/admin.rs:53`), and
  the arg-less RPC HELLO can never trigger it. Critically, **zero SDKs parse `[code]` or `NOAUTH`
  into a typed error today** (analysis T-003; ARCHITECTURE.md §3.2 "Typed error-code parsing —
  donor: none").
- **Impact**: the divergence is real on the wire (a Vectorizer auth failure is `[401] …`-shaped, a
  Nexus one is `WRONGPASS …`-shaped) but it is *unconsumed* — every client today treats the whole
  string as opaque. That is the normalization opportunity: because no client depends on the specific
  grammar yet, converging every server onto **one** grammar has almost no client-visible blast radius
  beyond human log readers. Lexum's `both` proves the superset is already the intended landing zone;
  the remaining work is making every server *emit* the one canonical form rather than each client
  *tolerate* two (§2 BN-011). The canonical superset's recognized token set must include `NOPERM` —
  Synap emits it today and no spec or profile mentions it.
- **Confidence**: high.

## 1.4 Dimension 3 — Server push

### BN-005 — Push is already wire-uniform; the only divergence is whether a server ever *emits* a push frame

- **Evidence**: the `PUSH_ID = u32::MAX` reservation is identical in all four profiles and frozen at
  the wire level for every one of them (SPEC-001 WIRE-005; `Nexus/docs/specs/rpc-wire-format.md:220-232`).
  What differs is a single boolean: `push: enabled` for Synap (it ships `SUBSCRIBE`, emitting frames
  with id `u32::MAX` — analysis §1.2 table) vs `push: reserved` for Nexus/Vectorizer/Lexum (they
  reserve the id, refuse it from clients, and never emit — F-011 push row; SPEC-002 PRO-031).
- **Impact**: unlike handshake, this is **not a dialect divergence** — every conformant client on
  every profile already must (a) never send `PUSH_ID` as a request id and (b) route an inbound
  `PUSH_ID` frame to a push hook rather than a pending call (SPEC-003 CLT-060). The reserved-vs-enabled
  flag only decides whether the *server* uses the channel. That means push is the cheapest dimension
  to "normalize": make the client push contract uniform (Thunder's floor already does), and the
  divergence degrades from a protocol difference to a per-product *capability* — the same way "Nexus
  has `CYPHER` and Synap doesn't" is a capability, not a dialect (§2 BN-010).
- **Confidence**: high.

## 1.5 Dimension 4 — Frame cap and in-flight bound

### BN-006 — Synap is the lone cap outlier at 512 MiB — but its own SDK already caps at 64, so the effective family ceiling is already uniform

- **Evidence**: `max_frame_bytes` is 64 MiB for Nexus/Vectorizer/Lexum and **512 MiB for Synap**
  (`synap.yaml:7`, "matches synap-protocol MAX_FRAME_SIZE"). But the Synap crate constant and the
  Synap *client* disagree: the Synap Rust SDK **hardcodes 64 MiB** against the crate's 512 MiB
  (analysis T-005 point 3; `01-current-state.md:35,97`). Nexus's cap is operator-configurable
  (`rpc.max_frame_bytes` / `NEXUS_RPC_MAX_FRAME_BYTES`,
  `Nexus/docs/specs/rpc-wire-format.md:264-283`); Vectorizer's is hardcoded 64 (F-011). Separately,
  `max_in_flight` is 1024 for Nexus vs 256 for the others (`nexus.yaml:8`) — a per-connection
  backpressure bound, invisible on the wire.
- **Impact**: the cap looks like a divergence but is nearly a non-issue: Synap's *real* client-side
  ceiling is already 64 MiB (any frame between 64 and 512 MiB would be refused by Synap's own SDK
  today), so converging the default to 64 MiB and making it operator-configurable everywhere — which
  Thunder already mandates in WIRE-020 — changes nothing observable for existing Synap traffic while
  giving an operator the escape hatch to restore 512 if a genuine large-payload path needs it.
  `max_in_flight` is a tuning knob, not observable protocol behavior; a single configurable default
  normalizes it with zero interop impact. This is among the cheapest dimensions to normalize (§2 BN-008).
- **Confidence**: high.

## 1.6 Dimension 5 — TLS

### BN-007 — RPC TLS runs in **zero** products today (one specs it, unwired); the divergence is a *gap to fill uniformly*, not a conflict to reconcile

- **Evidence**: the registry credits Vectorizer with `optional_rustls` (`vectorizer.yaml:10`), but
  the source shows it is **spec'd, not wired**: the spec describes `config.rpc.tls.cert_path` /
  `key_path` (`Vectorizer/docs/specs/VECTORIZER_RPC.md:293-296`), yet the RPC listener binds plain
  TCP and splits the raw stream with no `TlsAcceptor` anywhere in `protocol/rpc/`
  (`vectorizer-server/src/protocol/rpc/server.rs:62,92`), and `RpcConfig` exposes only
  `enabled`/`host`/`port` — the TLS keys the spec references do not exist
  (`vectorizer/src/config/vectorizer.rs:117-131`); the `tokio-rustls` dependency serves the REST
  side. Nexus explicitly does **not** ship native RPC TLS in 1.0, documenting an LB/sidecar posture
  instead ("Native TLS for RPC is not shipped in 1.0.0 … tracked for V2",
  `Nexus/docs/OPERATING_RPC.md:57-71`), and its `RpcConfig` has no TLS fields; Synap is `off`; Lexum
  reserves the config keys (`lexum.yaml:10` `reserved_config`). Thunder already specs the uniform
  implementation: server-side `SRV-040` and client-side `FR-29`, both feature/config-gated on
  `tokio-rustls`/native.
- **Impact**: TLS is unique among the five dimensions in being **additive** — nobody has a
  *conflicting* TLS behavior to migrate off of; in fact nobody has a running RPC TLS behavior at
  all. The capability exists in-family only as Vectorizer's spec design, which makes Thunder's
  `SRV-040`/`FR-29` layer the family's **first running implementation**, shipped identically to all
  four. No deployed client breaks (an off-by-default encrypted-transport option is backward
  compatible), and the "must land before Vectorizer's swap if a deployment already runs rustls"
  ordering worry dissolves — with no config keys, no deployment *can* have it on (§2 BN-009). The
  registry cell is an errata to fix (BN-023).
- **Confidence**: high.

## 1.7 What the primary-source sweep corrected — registry errata

### BN-023 — Three profile-registry cells, one corpus vector and the SPEC-002 prose mischaracterize the products; fix them in one coordinated errata commit, independent of any normalization decision

- **Evidence** (all found by sweeping the product sources for this analysis; file:line anchors in
  BN-003/BN-004/BN-007):
  1. **`synap.yaml:4` `handshake: none`** — Synap's RPC path has an `AUTH` handler, a `NOAUTH` gate
     and a `NOPERM` admin ACL behind `require_auth`, backed by the shared `UserManager`
     (`synap_rpc/server.rs:170-249`). The truthful cell is `auth_command` with `hello_style: null`
     (no HELLO exists); `none` describes only the `require_auth = false` posture.
  2. **`nexus.yaml:5` `hello_style: positional_version`** — the RPC `HELLO` takes **no arguments**
     (`rpc/dispatch/admin.rs:54-77`; spec row `HELLO | [] | Map`); positional `[Int(1)]` is the
     RESP3 HELLO. The corpus vector `handshake-nexus-hello-request.yaml` pins the wrong request
     shape — it is *tolerated* (the handler ignores args) but documents RESP3, not RPC.
  3. **`vectorizer.yaml:10` `tls: optional_rustls`** — spec'd, never wired (`RpcConfig` has no TLS
     keys); the truthful cell is `reserved_config`-class, like Lexum's, until the uniform Thunder
     layer lands.
  4. **`NOPERM` is unmodeled** — Synap emits it today (`server.rs:243-245`); neither SPEC-002/003
     nor any profile lists it among the recognized auth-family tokens.
- **Impact**: two of these are not just documentation errors. (1) means a Thunder client on the
  `synap` profile **cannot authenticate against a `require_auth` Synap at all** — the `None`
  handshake branch never sends `AUTH` — a real functional gap in the profile-driven floor. (2) means
  `thunder-client`'s AuthCommand branch would emit a HELLO shape no Nexus RPC server documents. The
  fix is one coordinated commit in the Thunder repo: the three YAML cells, the corpus vector, the
  SPEC-002 PRO-001 prose, and the `Profile` constants + pinning tests in all four language packages.
  **Sequencing**: apply after the in-flight T3 packages land, so the YAMLs and every language's
  constants change together; it is registry correctness, owed regardless of whether normalization
  (N0–N4) ever runs.
- **Confidence**: high — every item is anchored to primary source read on 2026-07-17.

## 1.8 The shape of the problem

Laid end to end, the five dimensions are not equally hard, and the inventory already reveals why:

| Dimension | Divergence class | Blast radius of eliminating it |
|---|---|---|
| Caps + in-flight | Default value only; Synap SDK already at 64 | ~None (BN-006) |
| TLS | Missing capability in 3 of 4 | ~None — additive, off by default (BN-007) |
| Push | Already wire-uniform; only server emission differs | ~None — client floor already uniform (BN-005) |
| Error grammar | Two grammars, but unconsumed by any client | Small — server log strings change; superset already Lexum's target (BN-004) |
| Handshake | Three incompatible first-frame models | **Large — a real protocol change per product** (BN-003) |

The strategic reading is in the last column: **four of the five dimensions can be normalized at
near-zero interop cost**, because they are either default values, additive capabilities, or already
uniform at the wire. The entire difficulty of "make them all speak exactly the same way" concentrates
in **one** dimension — the handshake — and even there the difficulty is unevenly distributed across
the four products (§3). §2 takes each dimension and asks: what is the single canonical behavior, and
what does it cost to converge on it?

---

Next: [§2 — Feasibility by dimension](02-feasibility-by-dimension.md).
