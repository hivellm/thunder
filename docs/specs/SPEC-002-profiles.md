# SPEC-002 — Protocol Profiles

| | |
|---|---|
| **Status** | Draft — registry schema freezes at G1 |
| **Phase / tasks** | Phase 0 · T0.3 + Phase 1 · T1.4/T1.5 + Phase 2 · T2.5 ([DAG](../DAG.md)) |
| **PRD requirements** | FR-10..FR-12 |
| **Requirement prefix** | `PRO-` |
| **Source** | Divergence table F-011 (`Lexum/docs/analysis/hivellm-rpc/02-implementations.md`); analysis [§2 T-010](../analysis/02-module-design.md), [§5.3 T-023](../analysis/05-protocol-crate-dissolution.md) |

Requirement IDs `PRO-xxx`. A **profile** is the declarative description of how one product uses
the shared wire — everything the family's three servers legitimately do differently, expressed as
data so that one module serves all of them without forks.

---

## 1. The profile model

- **PRO-001** [P0] A `Profile` SHALL carry exactly these dimensions:

  | Field | Type / values | Semantics |
  |---|---|---|
  | `handshake` | `None` \| `AuthCommand` \| `HelloMandatory` | `None`: no RPC-layer handshake at all — no registered family profile uses it (available for custom profiles, PRO-020). `AuthCommand`: `HELLO` optional, `AUTH [api_key]`/`[user, pass]`/`[password]`, pre-auth allowlist `PING/HELLO/AUTH/QUIT` (Nexus, Synap). `HelloMandatory`: first frame MUST be `HELLO` carrying credentials (Vectorizer/Lexum) |
  | `hello_style` | `NotUsed` \| `ArgLess` \| `MapPayload` | `NotUsed`: the profile has no `HELLO` command (Synap). `ArgLess`: `HELLO` takes **no arguments**; reply is a metadata Map `{server, version, proto, id, authenticated}` (Nexus). `MapPayload`: Map with `version`, `token` **or** `api_key`, `client_name`; reply carries `capabilities` (Vectorizer/Lexum) |
  | `push` | `Reserved` \| `Enabled` | `Reserved`: server refuses client `PUSH_ID`, never emits push. `Enabled`: push frames delivered to the client hook (Synap SUBSCRIBE) |
  | `max_frame_bytes` | u32, default 64 MiB | Frame cap (WIRE-020); Synap profile MAY set 512 MiB to match its crate constant |
  | `max_in_flight` | u32 | Per-connection request bound (Nexus 1024, Vectorizer 256) |
  | `error_codes` | `None` \| `Resp3Prefixes` \| `BracketCode` \| `Both` | Which prefix conventions the client parses into typed errors (CLT-050). `Resp3Prefixes` recognizes `ERR`/`NOAUTH`/`WRONGPASS`/`NOPERM` |
  | `tls` | `Off` \| `Optional` \| `Reserved` | Transport security policy. No family product runs RPC TLS today — Vectorizer's is spec'd but unwired, so its profile is `Reserved` (BN-023) |

- **PRO-001a** [P0] **The handshake is a *shape*, not an auth policy.** A profile fixes what frames a
  correct client sends first; it does NOT decide whether a deployment demands credentials. Both
  family products that authenticate on the RPC path expose that as their own config toggle (Nexus
  `auth_required`, Synap `require_auth`), and an open Synap deployment is a real, supported posture.
  Therefore:
  - clients under `AuthCommand` with **no credentials configured** SHALL send no `AUTH` frame — the
    correct behavior against an open deployment;
  - servers SHALL take auth enforcement from **deployment config** (`ListenerConfig::auth_required`),
    never infer it from the handshake variant.

  Conflating the two is what left the `synap` profile — recorded as `handshake: none` because Synap
  *can* run open — unable to authenticate at all against a `require_auth` Synap (BN-023).

- **PRO-002** [P0] Profile fields SHALL have defaults such that adding a field is a **minor**
  release (older data files remain valid).
- **PRO-003** [P0] Profiles are **data, not behavior**: no profile may alter wire bytes
  (SPEC-001 governs bytes unconditionally). A profile selects among behaviors Thunder already
  implements.

## 2. The family registry

- **PRO-010** [P0] Family profiles SHALL be defined as data files in `conformance/profiles/*.yaml`
  and materialized (codegen or reviewed hand-written constants) identically in every language:
  `Profile::synap()`, `Profile::nexus()`, `Profile::vectorizer()`, `Profile::lexum()`
  (and `Profiles.synap` etc. in TS/Python/C#).
- **PRO-011** [P0] Registered values at 1.0:

  | Profile | Scheme | RPC port | handshake | hello_style | push | error_codes | tls |
  |---|---|---|---|---|---|---|---|
  | synap | `synap://` | 15501 | AuthCommand | NotUsed | Enabled | Resp3Prefixes | Off |
  | nexus | `nexus://` | 15475 | AuthCommand | ArgLess | Reserved | Resp3Prefixes | Off |
  | vectorizer | `vectorizer://` | 15503 | HelloMandatory | MapPayload | Reserved | BracketCode | Reserved |
  | lexum | `lexum://` | 17001 | HelloMandatory | MapPayload | Reserved | Both | Reserved |

  Three cells were corrected on 2026-07-17 after the products' RPC listeners were read directly
  (BN-023 errata, `docs/analysis/behavioral-normalization/`): `synap.handshake` was `None` (its RPC
  path does authenticate — `AUTH` + `NOAUTH` gate + `NOPERM` ACL behind `require_auth`; what it
  lacks is a `HELLO` handler, hence `hello_style: NotUsed`); `nexus.hello_style` was
  `PositionalVersion` (the arg-less RPC `HELLO` is not the positional RESP3 one); `vectorizer.tls`
  was `Optional` (spec'd, never wired).

- **PRO-012** [P0] The registry also binds each profile's **URL scheme and default port** consumed
  by the endpoint parser (CLT-070). Scheme registration is data-driven; products do not subclass
  the parser.
- **PRO-013** [P0] The conformance suite SHALL exercise every registered profile (handshake vector
  group + behavioral floor per profile); a registry typo MUST fail CI in all languages (TST-060).
- **PRO-014** [P0] `error_codes = BracketCode` SHALL cause clients to parse a leading
  `"[<code>] "` prefix into a structured `code` field; `Resp3Prefixes` maps
  `NOAUTH`/`WRONGPASS` → typed auth error and `ERR ` → generic server error; `Both` composes them.

## 3. Custom profiles

- **PRO-020** [P0] Public `Profile` construction SHALL remain available in every language — a new
  or external product MUST be able to use Thunder without a Thunder release (analysis T-023).
- **PRO-021** [P1] Adding a product to the shipped registry is a **minor** release and requires:
  the YAML entry, a handshake corpus vector if the style is new, and a floor-test run under the
  new profile.

## 4. Server-side enforcement

- **PRO-030** [P0] `thunder::server` SHALL enforce the profile: `HelloMandatory` rejects any
  non-`HELLO` first frame with the profile's error convention; `AuthCommand` applies the pre-auth
  allowlist; `None` skips auth gating entirely.
- **PRO-031** [P0] `push = Reserved` servers SHALL refuse client frames with `PUSH_ID` and never
  emit push frames; `Enabled` delegates push emission to the product dispatch layer.
