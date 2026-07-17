# Proposal: phase2_vectorizer-swap

## Why
vectorizer-protocol is the second of the three identical wire copies, and its release pain is documented in-repo: PUBLISHING.md prescribes publish-protocol-first-then-SDK ordering on every wire-touching release (analysis T-021). Risk is the lowest of the three swaps - Thunder's client was derived from Vectorizer's (the only in-family Rust client with true demux), and this swap validates the HelloMandatory/MapPayload profile path end to end.

## What Changes
Same §5.5 recipe as Nexus. Server: the Vectorizer RPC listener moves onto thunder-server with Profile::vectorizer(); the HELLO capabilities reply is supplied through the dispatch capabilities() hook (SRV-014), and the TLS decision recorded at T0 is honored through the profile's tls dimension. Rust SDK: dependencies become thunder-wire + thunder-client from crates.io with the one-line type alias; the existing thin product-side pool wrapper over Thunder clients is kept until CLT-080 lands (SPEC-003 swap note), so the swap never blocks on the pool requirement. Vectorizer's golden vector tests are retained as a transition double-check. Exit proof: full suite + corpus green, `cargo publish --dry-run` clean.

## Impact
- Governing spec: SPEC-006 (PKG-020..022, PKG-030) - docs/specs/SPEC-006-packaging-release.md; SPEC-002 (PRO-011 vectorizer row); SPEC-003 (CLT-080 swap note)
- PRD requirements: FR-61
- DAG: T2.2 (gate G2); depends on G1; feeds T2.4
- Affected code: e:\HiveLLM\Vectorizer - server RPC listener, sdks/rust (registry deps + alias + pool wrapper); no Thunder code changes expected
- Breaking change: NO (bytes identical; SDK public API preserved via the alias, pool surface unchanged)
- User benefit: enforced caps/timeouts from Thunder's floor; Vectorizer releases lose the protocol-publish choreography
