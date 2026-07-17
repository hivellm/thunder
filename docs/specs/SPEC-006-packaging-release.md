# SPEC-006 — Packaging, Release & `-protocol` Dissolution

| | |
|---|---|
| **Status** | Draft — names freeze at G0 |
| **Phase / tasks** | Phase 0 · T0.1/T0.2 + Phase 2 · T2.1–T2.4 + Phase 3 · T3.4–T3.6 ([DAG](../DAG.md)) |
| **PRD requirements** | FR-60..FR-63; NFR-04, NFR-06 |
| **Requirement prefix** | `PKG-` |
| **Source** | Analysis [§2.5 T-011/T-012](../analysis/02-module-design.md), [§5 T-021..T-024](../analysis/05-protocol-crate-dissolution.md) |

Requirement IDs `PKG-xxx`. Two products in one spec because they are inseparable: what Thunder
publishes, and what the family **stops** publishing because of it.

---

## 1. Repository layout

- **PKG-001** [P0] Monorepo:

  ```
  Thunder/
  ├── docs/            # spec (transplanted), PRD, DAG, ROADMAP, specs/, analysis/
  ├── conformance/     # vectors/ + profiles/ + fuzz seeds (language-neutral)
  ├── rust/            # thunder-wire, thunder-client, thunder-server, thunder-bench
  ├── typescript/      # @hivehub/thunder
  ├── python/          # hivellm-thunder
  └── csharp/          # HiveLLM.Thunder
  ```

- **PKG-002** [P0] CI matrix: Rust fmt + clippy `-D warnings` + tests on Linux/macOS/Windows;
  tsc + eslint + vitest; ruff + pytest; `dotnet build -warnaserror` + test. Corpus lanes per
  TST-020/021. Family quality-gate order: type-check → lint → tests.

## 2. Published packages

- **PKG-010** [P0] Registry artifacts (names confirmed at T0.2; fallbacks recorded in the same
  decision):

  | Registry | Package(s) |
  |---|---|
  | crates.io | `thunder-wire`, `thunder-client`, `thunder-server` |
  | npm | `@hivehub/thunder` |
  | PyPI | `hivellm-thunder` (import `thunder_rpc`) |
  | NuGet | `HiveLLM.Thunder` |

- **PKG-011** [P0] **One release train**: all packages version together (lockstep semver); a
  release publishes every registry from one tag. Wire v1 being frozen makes trains rare by
  construction.
- **PKG-012** [P0] Semver policy: new commands/products never involve Thunder; profile field with
  default, new corpus vectors, new language port = **minor**; decode-tolerance removal, floor
  default changes, public API breaks = **major**; canonical byte changes = never (NFR-01).
- **PKG-013** [P0] `thunder-wire` has no tokio dependency; `thunder-client`/`-server` depend on
  `thunder-wire` by exact `=` version within the train.

## 3. Consumption rules (family)

- **PKG-020** [P0] Family servers and SDKs consume **released registry versions only** — never git
  paths, never vendored copies (PRD NFR-06). This is what makes silent drift structurally
  impossible.
- **PKG-021** [P0] Product SDK swaps MUST NOT change the SDK's public API (PRD NFR-04). Product
  value-type names survive as one-line aliases in the SDK
  (`pub type NexusValue = thunder_wire::Value;`).
- **PKG-022** [P0] Each swapped Rust SDK proves `cargo publish --dry-run` with zero path
  dependencies and no product-protocol package — the gate-G2 criterion.

## 4. Dissolution of the per-product `-protocol` crates

- **PKG-030** [P0] Per product (Nexus, Vectorizer, Synap), in one PR each:
  1. Server replaces `<product>-protocol` deps with `thunder-wire`/`thunder-server`; non-RPC
     residue relocates in-repo (`resp3/` → `nexus-server` internal; `envelope.rs` + `resp3/` →
     `synap-server` internal) and is never published again.
  2. Rust SDK depends on `thunder-wire`/`thunder-client` from crates.io + the PKG-021 alias.
  3. A **terminal shim** version of `<product>-protocol` is published: contents are
     `#[deprecated]` re-exports of `thunder-wire` with the old type names, README deprecation
     notice pointing here. For external downstream only — in-repo consumers never route through it.
  4. `crates/<product>-protocol` is deleted from the workspace; the product's release pipeline
     drops its protocol-publish step permanently.
- **PKG-031** [P0] The shim is the crate's **last** version, ever (crates.io does not delete;
  shim-then-archive is the terminal state).
- **PKG-032** [P0] Synap ordering constraint: the server ships bin-`Bytes` emission (WIRE-010)
  **before or with** its SDK swap; old Synap SDKs keep working because they already decode both
  forms, and Thunder decodes their legacy form via WIRE-011.

## 5. Non-Rust SDK swaps

- **PKG-040** [P0] The nine transports (TS/Py/C# × three products) are replaced by the Thunder
  packages; per-SDK codec/transport source files are **deleted**, not kept as dead code. Expected
  net deletion ≈ 11k LOC (analysis T-001).
- **PKG-041** [P0] Product SDK version bumps for the swap are **minor** (internals-only,
  PKG-021); release notes state the behavioral upgrades (caps enforced, timeouts, pipelining
  where missing) explicitly.

## 6. Go fast-follow

- **PKG-050** [P1] `github.com/hivellm/thunder-go` (module tag releases), corpus loader included,
  `vmihailenco/msgpack` v5 with compact ints — enters the release train as a fifth lane.
