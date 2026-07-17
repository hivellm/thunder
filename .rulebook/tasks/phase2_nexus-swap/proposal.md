# Proposal: phase2_nexus-swap

## Why
nexus-protocol exists only to feed the Rust SDK its wire types, and because crates.io rejects path-only dependencies, every wire-touching Nexus release means publish-protocol-then-publish-SDK choreography for bytes that are 95% identical across the family (analysis T-021). The crate also drags server-internal RESP3 parsing onto a public registry. Meanwhile the Nexus Rust SDK's own transport is mutex single-flight - it lacks the pipelining Thunder's client gives for free (T-003).

## What Changes
First product swap of the dissolution recipe (§5.5). Server: the nexus-server RPC listener moves onto thunder-server with Profile::nexus(), a dispatch adapter wrapping the existing dispatch/ modules unchanged; nexus-protocol/src/resp3/ relocates into nexus-server as an internal module and is never published again. Rust SDK: dependencies become thunder-wire + thunder-client from crates.io, with the one-line alias `pub type NexusValue = thunder_wire::Value;` preserving the public API; the SDK gains pipelining via thunder-client's demux; the NEXUS_SDK_TRANSPORT factory stays product-side. Exit proof: full Nexus suite + corpus green, and `cargo publish --dry-run` shows zero path dependencies (PKG-022).

## Impact
- Governing spec: SPEC-006 (PKG-020..022, PKG-030) - docs/specs/SPEC-006-packaging-release.md; SPEC-002 (PRO-011 nexus row)
- PRD requirements: FR-61; NFR-04
- DAG: T2.1 (gate G2); depends on G1; feeds T2.4
- Affected code: e:\HiveLLM\Nexus - nexus-server (listener + internal resp3 module), sdks/rust (registry deps + alias); no Thunder code changes expected
- Breaking change: NO (bytes structurally identical; SDK public API preserved by the alias per NFR-04)
- User benefit: SDK gains pipelining, enforced frame caps and timeouts; Nexus releases lose the protocol-publish choreography
