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
  ├── rust/            # thunder (wire+client+server, feature-gated) + thunder-bench (internal, unpublished)
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

  | Registry | Package | Availability (checked 2026-07-17) |
  |---|---|---|
  | crates.io | **`thunder-rpc`** (one crate; `wire` always compiled, `client`/`server` default-on features; **lib name stays `thunder`**, so `use thunder::…` is unchanged) | `thunder` is **TAKEN** — fallback applied |
  | npm | `@hivehub/thunder` | free |
  | PyPI | `hivellm-thunder` (import `thunder_rpc`) | free |
  | NuGet | `HiveLLM.Thunder` | free |

  **The crates.io name collided and the fallback is now the name.** `thunder` is a 2018
  CLI-boilerplate crate (last release 0.3.1, 9,478 downloads). crates.io does not recycle
  published names, so the fallback this task pre-approved — `thunder-rpc` — is the registry
  identity. `[lib] name = "thunder"` keeps the collision confined to the registry: `cargo add
  thunder-rpc` still gives `use thunder::`, so no product, doc or example changes. Verified with
  `cargo publish --dry-run` (25 files, 66.7 KiB).

  The Rust side is a **single** crate, not three: `thunder-wire`/`thunder-client`/`thunder-server`
  always versioned and released together, so publishing them separately only added release
  choreography. They are now feature-gated modules of `thunder` (superseding the original
  three-name reservation — see `.rulebook/decisions/2026-07-17-registry-names.md`).

- **PKG-011** [P0] **One release train**: all packages version together (lockstep semver); a
  release publishes every registry from one tag. Wire v1 being frozen makes trains rare by
  construction. (On crates.io this is now literally one crate, not three lockstep crates.)
  Lanes may publish from the tagged commit by other means, which the requirement permits —
  the constraint is *one tag, one version*, not *one mechanism*: **Go** and **PHP** release
  from a VCS tag, and **NuGet** is published by hand after its automated job returned 403 on
  three consecutive releases. All remain gated: `release.yml` runs their checks and the
  tag-vs-manifest check on the tagged commit before any lane ships.
  Two lanes authenticate by **OIDC trusted publishing** rather than a stored credential —
  PyPI, and npm since it gained the capability. This is the preferred mechanism where a
  registry offers it: there is no token to leak, rotate, or find in a product repo, and it is
  what made npm automatable at all. The `@hivehub` org requires an OTP on publish, which no
  *stored* credential can produce; an OIDC identity is not a stored credential.
- **PKG-012** [P0] Semver policy: new commands/products never involve Thunder; profile field with
  default, new corpus vectors, new language port = **minor**; decode-tolerance removal, floor
  default changes, public API breaks = **major**; canonical byte changes = never (NFR-01).
- **PKG-013** [P0] Within the `thunder-rpc` crate (lib `thunder`) the `wire` layer carries no tokio dependency; the
  `client` and `server` features each enable `tokio`. A pure-wire consumer builds with
  `default-features = false`; a client-only SDK with `features = ["client"]`; a server with
  `["server"]`. One crate version covers all layers (no intra-crate `=` pinning to maintain).

## 3. Consumption rules (family)

- **PKG-020** [P0] Family servers and SDKs consume **released registry versions only** — never git
  paths, never vendored copies (PRD NFR-06). This is what makes silent drift structurally
  impossible.
- **PKG-021** [P0] Product SDK swaps MUST NOT change the SDK's public API (PRD NFR-04). Product
  value-type names survive as one-line aliases in the SDK
  (`pub type NexusValue = thunder::Value;`).
- **PKG-022** [P0] Each swapped Rust SDK proves `cargo publish --dry-run` with zero path
  dependencies and no product-protocol package — the gate-G2 criterion.

## 4. Dissolution of the per-product `-protocol` crates

- **PKG-030** [P0] Per product (Nexus, Vectorizer, Synap), in one PR each:
  1. Server replaces `<product>-protocol` deps with `thunder-rpc` (features `server`); non-RPC
     residue relocates in-repo (`resp3/` → `nexus-server` internal; `envelope.rs` + `resp3/` →
     `synap-server` internal) and is never published again.
  2. Rust SDK depends on `thunder-rpc` (features `client`, `default-features = false`) from crates.io — imported as `thunder`
     + the PKG-021 alias.
  3. A **terminal shim** version of `<product>-protocol` is published: contents are
     `#[deprecated]` re-exports of `thunder` (its `wire` layer) with the old type names, README
     deprecation notice pointing here. For external downstream only — in-repo consumers never route
     through it.
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
- **PKG-051** [P1] `hivellm/thunder` on Packagist (VCS tag releases, so no push step — but the tag
  must exist, as the Go lane learned), corpus loader included, `rybakit/msgpack` ^0.9 — enters the
  release train as a sixth lane. Wire layer and client both ship. Its source of truth is
  `github.com/hivellm/thunder-php`, mirrored into the monorepo as a submodule; the corpus tests
  skip in the standalone checkout and run for real upstream.
  PHP has no threads and no event loop, so its client demultiplexes on read rather than from a
  background reader, and exposes `send`/`collect` for multiple-in-flight. The observable contract
  of CLT-010/013/060 holds — responses matched by id, unknown ids dropped, pushes routed — and the
  deviation is documented in the lane's README. A lane MAY deviate where a runtime makes the
  reference design impossible; it MUST NOT claim a parity it does not have.
