# §5 — Dissolving the Per-Product `-protocol` Crates

> The specific pain this section resolves: every product carries a `crates/<product>-protocol` crate whose only reason to exist is that the Rust SDK needs the wire types — and because the SDK is published to crates.io, the protocol crate **must be published too**, forcing a per-product release choreography for code that is 95% identical across the family. The elegant end state is not "re-export Thunder from nexus-protocol forever"; it is **no per-product protocol package at all**.

## 5.1 The pain, precisely

### T-021 — The `-protocol` crates exist to feed the SDKs, and publishing the SDK forces publishing the crate — three times, for the same bytes

- **Evidence**: crates.io rejects path-only dependencies, so every published SDK drags its protocol crate to the registry: `Nexus/sdks/rust/Cargo.toml:32` — `nexus-protocol = { path = …, version = "2.5.0" }` (published as `nexus-protocol` 2.5.0 alongside `nexus-graph-sdk`); `Vectorizer/sdks/PUBLISHING.md:360-381` documents the choreography explicitly — `vectorizer-protocol` must be **published first**, then the SDK; `Synap/sdks/rust/Cargo.toml:15` — `synap-protocol = { path = …, version = "1.0.0" }`, same pattern. Every wire-touching release therefore means: bump + publish the protocol crate, wait, bump the SDK's pinned version, publish the SDK — per product.
- **Aggravator**: the crates are not even pure wire. `nexus-protocol` also carries `src/resp3/` (RESP3 parser/writer); `synap-protocol` carries `envelope.rs` (HTTP envelope) and `resp3/` (880 LOC) next to `synap_rpc/`. Products are publishing **server-internal parsing code to a public registry** just to hand their SDK ~600 lines of shared types.
- **Impact**: three public packages to version, document, and release-order — for one frozen protocol. This is the maintenance tax the user feels on every release.
- **Confidence**: high.

## 5.2 The resolution: invert the dependency

### T-022 — Thunder publishes the protocol once; products stop publishing protocol packages entirely

End state per consumer (no `-protocol` crate remains in any product workspace):

| Consumer | Depends on (from the registry) | What replaces the old crate's contents |
|---|---|---|
| Product **server core** (Rust) | `thunder-wire` + `thunder-server` | RPC wire → Thunder. Non-RPC residue relocates in-repo: `resp3/` → a server-internal module (Nexus SDK never consumed it — its transport imports only `nexus_protocol::rpc`, `sdks/rust/src/transport/rpc.rs:11-12`; Synap's Rust SDK hand-rolls its own RESP3 parser, `transport/mod.rs`); `envelope.rs` → `synap-server` internal. Internal modules are never published. |
| Product **Rust SDK** | `thunder-wire` + `thunder-client` | The product alias is one line in the SDK itself: `pub type NexusValue = thunder_wire::Value;` — source compatibility for SDK users without any intermediate crate. |
| Product **TS/Py/C# SDKs** | `@hivellm/thunder` / `hivellm-thunder` / `HiveLLM.Thunder` | Today's in-package transport copies are deleted (they were never separate packages — no dissolution needed, just the P3 swap). |
| **New projects** (Lexum, …) | Thunder packages directly | The planned `lexum-protocol` is never created (T-019). |

- **Impact**: a wire-touching change becomes **one release train** (Thunder's four packages, one version) instead of three protocol-then-SDK choreographies plus twelve hand-ports. Product releases stop having a protocol-publish step at all — the SDK pins `thunder-wire = "1"` and publishes independently, whenever the product wants. And because wire v1 is frozen (T-016), Thunder releases are rare by construction.
- **Confidence**: high.

## 5.3 The one thing left over: where the profile lives

Dissolving the crates leaves exactly one per-product protocol artifact: the ~10-line `Profile` (T-010). Three placement options were weighed:

| Option | Verdict |
|---|---|
| Per-product `-profile` crate | ❌ recreates the publishing problem in miniature |
| Inline the constant in server and SDK separately | Workable but lets the two copies of one product disagree |
| **Family profiles ship inside Thunder as data** | ✅ chosen |

### T-023 — Thunder hosts the family profile registry, generated from data files

- **Mechanics**: `conformance/profiles/{nexus,synap,vectorizer,lexum}.yaml` (scheme, default port, handshake style, error convention, caps, push policy) are the single documented source; codegen (or a build script) turns them into constants in every language — `Profile::nexus()` in Rust, `Profiles.nexus` in TS/Py/C#. Server and SDK of the same product import the same constant and **cannot disagree**. The public `Profile { … }` constructor remains, so an external or unreleased product is never blocked on a Thunder release.
- **Why this is sound here**: profiles are tiny, near-frozen data (ports and handshake styles change ~never), and the family is one organization — the same reasoning that makes well-known-port registries centralized. The conformance suite exercises every registered profile (§3.2), so a profile typo fails CI in all four languages at once.
- **Impact**: the last reason for a per-product protocol package disappears. A product's entire protocol footprint becomes: one YAML entry in Thunder + its command catalog in its own SDK.
- **Confidence**: high (medium on codegen vs hand-written constants — either is fine; the YAML is the contract).

## 5.4 Disposing of the already-published crates

### T-024 — Shim, deprecate, archive: the exit path for `nexus-protocol` / `vectorizer-protocol` / `synap-protocol`

1. **Final shim release** (one per crate): contents replaced by `pub use thunder_wire::…` with the old type names aliased (`pub type NexusValue = thunder_wire::Value;`) — anything downstream still compiling against the old crate keeps compiling. RESP3/envelope moves out in the same product release (5.2).
2. **Deprecation notice** in the shim's README + `#[deprecated]` on the re-exports, pointing at `thunder-wire`.
3. **Archive**: crates.io does not allow deletion — the shim stays as the terminal version; the crate is removed from the product workspace, and the product's release pipeline drops the protocol-publish step permanently.
4. In-repo consumers (server, SDK) never go through the shim — they switch to Thunder directly in the same PR (the shim exists only for *external* downstream, if any).

- **Impact**: the transitional "re-export for one deprecation cycle" mentioned in §2.5 is scoped precisely: it is the *terminal state of the old public artifact*, not a living layer products keep maintaining. Internal code paths reference Thunder only, from day one of the swap.
- **Confidence**: high.

## 5.5 Effect on the adoption plan

This sharpens §4 P2 — for each product, the swap PR is:

1. Server: replace `<product>-protocol` deps with `thunder-wire`/`thunder-server`; relocate `resp3`/`envelope` into the server crate.
2. Rust SDK: depend on `thunder-wire`/`thunder-client` from the registry; add the one-line type alias; delete the path dependency.
3. Publish the final shim version of `<product>-protocol` (T-024).
4. Delete `crates/<product>-protocol` from the workspace.

**Amended Gate G2 (additional criterion)**: the product's Rust SDK publishes with **zero path dependencies and zero product-protocol packages** — `cargo publish --dry-run` proves the SDK depends only on registry crates.
