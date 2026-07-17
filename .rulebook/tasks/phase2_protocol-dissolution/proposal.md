# Proposal: phase2_protocol-dissolution

## Why
The per-product -protocol crates exist only because published SDKs cannot use path dependencies, so every wire-touching release forces a protocol-then-SDK publish choreography - three times, for the same frozen bytes (analysis T-021). After T2.1-T2.3, no in-repo consumer references them: server and SDK already depend on Thunder directly. The elegant end state is not "re-export Thunder forever" but no per-product protocol package at all (T-022); what remains is disposing of the already-published crates cleanly (T-024).

## What Changes
For each of nexus-protocol, vectorizer-protocol, and synap-protocol: publish a TERMINAL shim version whose entire contents are `#[deprecated]` re-exports of thunder-wire under the old type names (e.g. `pub type NexusValue = thunder_wire::Value;`) plus a README deprecation notice pointing at thunder-wire - it exists for external downstream only; in-repo consumers never route through it. Then delete crates/<product>-protocol from the product workspace and drop the protocol-publish step from the product's release pipeline permanently. crates.io does not delete, so the shim is each crate's last version ever (PKG-031). This task also certifies the amended gate-G2 criterion: every product Rust SDK proves `cargo publish --dry-run` with zero path dependencies and no product-protocol package.

## Impact
- Governing spec: SPEC-006 (PKG-030, PKG-031) - docs/specs/SPEC-006-packaging-release.md
- PRD requirements: FR-61, FR-62
- DAG: T2.4 (gate G2); depends on T2.1 + T2.2 + T2.3
- Affected code: e:\HiveLLM\Nexus, e:\HiveLLM\Vectorizer, e:\HiveLLM\Synap - crates/<product>-protocol (terminal shim, then workspace deletion), each product's release pipeline
- Breaking change: NO (shims keep any external downstream compiling; deprecation is a warning, not a break)
- User benefit: a wire-touching change becomes one release train (Thunder's) instead of three protocol-then-SDK choreographies; product SDKs publish independently, whenever the product wants
