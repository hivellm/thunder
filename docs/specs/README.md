# Thunder — Specifications

This directory is the **implementation contract** for Thunder. The specs say *what to build*,
normatively. The feasibility study that explains *why* lives in [`docs/analysis/`](../analysis/README.md);
the wire bytes themselves are defined by the transplanted family spec in [`docs/spec/`](../spec/)
(created at DAG task T0.3) and are **not** re-specified here — SPEC-001 binds to them.

## How to navigate

| Question | Read |
|---|---|
| What are we building and why? Requirements and release criteria? | [**PRD.md**](../PRD.md) |
| What order does the work happen in? What blocks what? | [**DAG.md**](../DAG.md) |
| When does it ship? | [**ROADMAP.md**](../ROADMAP.md) |
| How does the whole system fit together? Which donor contributes what? | [**ARCHITECTURE.md**](../ARCHITECTURE.md) |
| How exactly does component X behave? | The SPEC for that component (below) |
| Why was it designed this way? | [`docs/analysis/`](../analysis/README.md) (findings T-001..T-026) |

Traceability chain: **PRD** requirement IDs (`FR-xx`, `NFR-xx`) → **DAG** tasks (`T<phase>.<n>`)
→ **SPEC** requirement IDs (`WIRE-xxx`, `PRO-xxx`, …) → tests (SPEC-005 conformance + per-crate suites).

## Specifications

| Spec | Scope | Prefix | Freeze event |
|---|---|---|---|
| [SPEC-001](SPEC-001-wire-format.md) — Wire Format Binding | Binding to frozen wire v1, value model, canonical encoding rules, legacy tolerances, caps, PUSH_ID | `WIRE-` | **Already frozen** (family v1) |
| [SPEC-002](SPEC-002-configuration.md) — Protocol Configuration | The config model, THE standard (`Config::standard()`), and how an application configures itself. No product registry — Thunder ships one standard and zero product knowledge | `PRO-` | The standard freezes at G1 |
| [SPEC-003](SPEC-003-client.md) — Client Contract | Connection lifecycle, handshakes, demux, timeouts, reconnect, push hook, typed errors, endpoint parsing, the uniform floor | `CLT-` | Public API shape freezes at G1 |
| [SPEC-004](SPEC-004-server.md) — Server (Rust) | Accept loop, writer task, semaphore, session auth, dispatch trait, metrics, TLS | `SRV-` | Dispatch trait freezes at G1 |
| [SPEC-005](SPEC-005-conformance.md) — Conformance & Testing | Golden-vector corpus format and groups, per-language loaders, reference cross-decode, pairwise fuzz, live interop, gate rules | `TST-` | Corpus format freezes at G1 |
| [SPEC-006](SPEC-006-packaging-release.md) — Packaging, Release & Dissolution | Package names/registries, release train, semver, `-protocol` crate dissolution, no-path-deps rule | `PKG-` | Names freeze at G0 |
| [SPEC-007](SPEC-007-benchmarks.md) — Benchmarks & the G5 Gate | Transport shootout (RESP3/Bolt/HTTP), scenario matrix, harness parity, artifacts, always-win gate, claims discipline | `BEN-` | Matrix freezes at G4 |

## Conventions

- RFC 2119 keywords (**MUST**, **SHALL**, **MUST NOT**, **SHOULD**, **MAY**) are normative.
- Every requirement carries a priority tag: **[P0]** required for 1.0.0 · **[P1]** fast-follow · **[P2]** future.
- Integers are little-endian unless stated otherwise.
- "The four languages" means Rust, TypeScript, Python, C#; requirements marked *(Rust)* apply only to the Rust stack.
- Wire-behavior requirements are testable by construction: each MUST map to at least one corpus vector or behavioral floor test (SPEC-005 `TST-090`).
