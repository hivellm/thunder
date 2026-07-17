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
  | `handshake` | `None` \| `AuthCommand` \| `HelloMandatory` | `None`: no RPC-layer auth (Synap v1). `AuthCommand`: `HELLO` optional, `AUTH [api_key]` or `[user, pass]`, pre-auth allowlist `PING/HELLO/AUTH/QUIT` (Nexus). `HelloMandatory`: first frame MUST be `HELLO` carrying credentials (Vectorizer/Lexum) |
  | `hello_style` | `PositionalVersion` \| `MapPayload` | `[Int(1)]` positional (Nexus) vs Map with `version`, `token` **or** `api_key`, `client_name` (Vectorizer) |
  | `push` | `Reserved` \| `Enabled` | `Reserved`: server refuses client `PUSH_ID`, never emits push. `Enabled`: push frames delivered to the client hook (Synap SUBSCRIBE) |
  | `max_frame_bytes` | u32, default 64 MiB | Frame cap (WIRE-020); Synap profile MAY set 512 MiB to match its crate constant |
  | `max_in_flight` | u32 | Per-connection request bound (Nexus 1024, Vectorizer 256) |
  | `error_codes` | `None` \| `Resp3Prefixes` \| `BracketCode` \| `Both` | Which prefix conventions the client parses into typed errors (CLT-050) |
  | `tls` | `Off` \| `Rustls`/platform-native | Transport security policy |

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

  | Profile | Scheme | RPC port | handshake | hello_style | push | error_codes |
  |---|---|---|---|---|---|---|
  | synap | `synap://` | 15501 | None | — | Enabled | Resp3Prefixes |
  | nexus | `nexus://` | 15475 | AuthCommand | PositionalVersion | Reserved | Resp3Prefixes |
  | vectorizer | `vectorizer://` | 15503 | HelloMandatory | MapPayload | Reserved | BracketCode |
  | lexum | `lexum://` | 17001 | HelloMandatory | MapPayload | Reserved | Both |

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
