# SPEC-002 — Protocol Configuration

| | |
|---|---|
| **Status** | Draft — the standard freezes at G1 |
| **Phase / tasks** | Phase 0 · T0.3 + Phase 1 · T1.4/T1.5 ([DAG](../DAG.md)) |
| **PRD requirements** | FR-10..FR-12 |
| **Requirement prefix** | `PRO-` |
| **Source** | Analysis [§2 T-010](../analysis/02-module-design.md), [§5.3 T-023](../analysis/05-protocol-crate-dissolution.md); the agnostic-config directive (2026-07-17) |

Requirement IDs `PRO-xxx`. A **config** is the declarative description of how **one application**
uses the shared wire, expressed as data so that one module serves every application without forks.

**Thunder ships one standard and zero product knowledge.** This spec used to be a registry: it
named four products, pinned their rows, and compiled them into every language. That is deleted. A
protocol library that must serve implementations which do not exist yet cannot ship a hardcoded
list of the ones that did, and a new application must never wait on a Thunder release for a row in
a table.

What replaces it: **`Config::standard()`** — the one canonical behavior — plus a knob for every
dimension, so an application that diverges says so **in its own repository**. Convergence is then
visible and per-application: delete overrides until only identity remains.

Thunder was born from three products' RPC implementations. That is history, recorded in
`docs/analysis/`; it is not shipped code. The verified facts about what each application currently
needs live there, as the reference for configuring it — knowledge, not a catalogue.

---

## 1. The config model

- **PRO-001** [P0] A `Config` SHALL carry exactly these dimensions:

  | Field | Type / values | Semantics |
  |---|---|---|
  | `handshake` | `None` \| `AuthCommand` \| `HelloMandatory` | `None`: no RPC-layer handshake at all. `AuthCommand`: `HELLO` optional, `AUTH [api_key]`/`[user, pass]`/`[password]`, pre-auth allowlist `PING/HELLO/AUTH/QUIT`. `HelloMandatory`: first frame MUST be `HELLO` carrying credentials — **the standard** |
  | `hello_style` | `NotUsed` \| `ArgLess` \| `MapPayload` | `NotUsed`: the application has no `HELLO` command. `ArgLess`: `HELLO` takes **no arguments**; reply is a metadata Map `{server, version, proto, id, authenticated}`. `MapPayload`: Map with `version`, `token` **or** `api_key`, `client_name`; reply carries `capabilities` — **the standard** |
  | `push` | `Reserved` \| `Enabled` | `Reserved`: server refuses client `PUSH_ID`, never emits push — **the standard**. `Enabled`: push frames delivered to the client hook (an application shipping a subscribe-style command) |
  | `max_frame_bytes` | u32, default 64 MiB | Frame cap (WIRE-020); an application MAY raise it for a genuine large-payload path |
  | `max_in_flight` | u32, default 256 | Per-connection request bound |
  | `error_codes` | `None` \| `Resp3Prefixes` \| `BracketCode` \| `Both` | Which prefix conventions the client parses into typed errors (CLT-050). `Resp3Prefixes` recognizes `ERR`/`NOAUTH`/`WRONGPASS`/`NOPERM` |
  | `tls` | `Off` \| `Optional` \| `Reserved` | Transport-security policy. Additive: a deployment turns it on, it is never a dialect |

- **PRO-001a** [P0] **The handshake is a *shape*, not an auth policy.** A config fixes what frames a
  correct client sends first; it does NOT decide whether a deployment demands credentials. Every
  real RPC implementation the family has shipped exposes enforcement as its own deployment toggle,
  and an open deployment is a real, supported posture. Therefore:
  - clients under `AuthCommand` with **no credentials configured** SHALL send no `AUTH` frame — the
    correct behavior against an open deployment;
  - servers SHALL take auth enforcement from **deployment config** (`ListenerConfig::auth_required`),
    never infer it from the handshake variant.

  Conflating the two is what once left an `AuthCommand` application modelled as `handshake: none`
  — because it *can* run open — unable to authenticate at all when it required credentials
  (BN-023). This principle governs the whole canonical-behavior program: SPEC-008 CAN-002.

- **PRO-002** [P0] Config fields SHALL have defaults (the standard's values) such that adding a
  field is a **minor** release — older data files and existing applications stay valid.
- **PRO-003** [P0] Configs are **data, not behavior**: no config may alter wire bytes
  (SPEC-001 governs bytes unconditionally). A config selects among behaviors Thunder already
  implements.

## 2. The standard

- **PRO-010** [P0] Thunder SHALL ship **exactly one** configuration — the standard — and **no**
  named, per-application configurations. No product name may appear in any language's library: not
  as a constant, not as a registry entry, not as the name of a behavior or shape.
- **PRO-011** [P0] The standard SHALL be defined once as language-neutral data in
  `conformance/standard.yaml` and materialized identically in every language as
  `Config::standard()` (`Config.Standard()` / `Config.standard()`):

  | Dimension | Standard | Why this value |
  |---|---|---|
  | `handshake` | `HelloMandatory` | the only shape that negotiates `proto` and advertises capabilities — what an evolving protocol needs |
  | `hello_style` | `MapPayload` | carries version, identity and credentials in one round trip; the reply carries capabilities |
  | `push` | `Reserved` | `PUSH_ID` is server→client only; *emitting* is a capability an application opts into |
  | `max_frame_bytes` | 64 MiB | checked before allocation (WIRE-020) |
  | `max_in_flight` | 256 | per-connection request bound |
  | `error_codes` | `Both` | a strict superset of both grammars, so it needs no negotiation |
  | `tls` | `Off` | an additive capability a deployment turns on, never a dialect |

  `scheme` and `default_port` have **no** standard value: identity is the application's, and
  Thunder has no opinion about it.

- **PRO-012** [P0] The endpoint parser SHALL take the caller's config and accept **that config's**
  scheme, resolving a missing port to its `default_port` (CLT-070/071). Thunder SHALL NOT carry a
  table of schemes to search: a scheme it has never heard of MUST work because the application
  configured it.
- **PRO-013** [P0] Every language SHALL pin its `Config::standard()` to `conformance/standard.yaml`
  in its default test run; a divergence between the standard and the data file MUST fail CI in all
  languages (TST-060). This is the one guarantee the deleted per-product registry legitimately
  provided — that the four implementations cannot disagree — and it survives without any product
  name.
- **PRO-014** [P0] `error_codes = BracketCode` SHALL cause clients to parse a leading
  `"[<code>] "` prefix into a structured `code` field; `Resp3Prefixes` maps
  `NOAUTH`/`WRONGPASS`/`NOPERM` → typed auth error and `ERR ` → generic server error; `Both`
  composes them and is the standard.

## 3. Application configuration

- **PRO-020** [P0] Public `Config` construction SHALL remain available in every language — both a
  builder over the standard and plain struct/record/object construction. An application, including
  one that does not exist yet, MUST be able to use Thunder without a Thunder release
  (analysis T-023).
- **PRO-021** [P0] An application SHALL express any divergence from the standard **in its own
  repository**, by overriding only the dimensions it actually differs on. Untouched dimensions stay
  standard, so convergence is "delete overrides until only identity remains" — visible per
  application, needing no coordinated release and no change to Thunder.
- **PRO-022** [P1] Adding a dimension to `Config` is a **minor** release (PRO-002 defaults keep
  older data valid). Changing a *standard value* is a **major**: it silently changes the behavior
  of every application that did not override it.

## 4. Server-side enforcement

- **PRO-030** [P0] `thunder::server` SHALL enforce the config: `HelloMandatory` rejects any
  non-`HELLO` first frame with the config's error convention; `AuthCommand` applies the pre-auth
  allowlist; `None` skips auth gating entirely. Whether credentials are *required* is deployment
  config, never the shape (PRO-001a).
- **PRO-031** [P0] `push = Reserved` servers SHALL refuse client frames with `PUSH_ID` and never
  emit push frames; `Enabled` delegates push emission to the product dispatch layer.
