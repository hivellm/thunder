# Proposal: phase2_synap-swap

## Why
Synap is the third wire copy and carries the one behavioral wire change of the entire program (NFR-04): its server emits Bytes as an int-array, ~50% bigger than bin on embedding payloads (probe T-029). Canonicalization is safe only if staged server-first (PKG-032): every deployed Synap SDK already special-cases and decodes both forms, and Thunder decodes the legacy int-array form indefinitely (WIRE-011), so no deployed combination can break. synap-protocol also publishes server-internal code (envelope.rs, resp3/) to a public registry just to feed the SDK ~600 lines of types (analysis T-021).

## What Changes
The §5.5 recipe plus the canonicalization. Server: synap-server moves onto thunder-server with Profile::synap() (handshake None, push Enabled); SUBSCRIBE-style push flows register against the per-connection PushSender, which stays valid past the registering request (SRV-013); synap-protocol's envelope.rs + resp3/ relocate into synap-server as internal modules. Bytes: the server starts emitting bin (WIRE-010) before or with the SDK swap; Thunder keeps decoding legacy int-arrays from old SDKs forever short of a major (WIRE-011/016). Rust SDK: dependencies become thunder-wire + thunder-client with the type alias - the SDK gains true demux over its hand-rolled transport. Old (pre-swap) SDKs are verified green against the new server. Exit proof: full suite + corpus green, `cargo publish --dry-run` clean.

## Impact
- Governing spec: SPEC-001 (WIRE-010/011) - docs/specs/SPEC-001-wire-format.md; SPEC-006 (PKG-030/032) - docs/specs/SPEC-006-packaging-release.md; SPEC-002 (PRO-011 synap row); SPEC-004 (SRV-013)
- PRD requirements: FR-02, FR-61; NFR-04
- DAG: T2.3 (gate G2); depends on G1; feeds T2.4
- Affected code: e:\HiveLLM\Synap - synap-server (listener, bin emission, internal envelope/resp3 modules), sdks/rust (registry deps + alias)
- Breaking change: NO (the only sanctioned behavioral wire change, staged server-first per PKG-032; all deployed SDKs decode both Bytes forms, and Thunder tolerates the legacy form via WIRE-011)
- User benefit: ~33% smaller embedding payloads on the wire; SDK gains true demux; push semantics preserved; Synap releases lose the protocol-publish choreography
