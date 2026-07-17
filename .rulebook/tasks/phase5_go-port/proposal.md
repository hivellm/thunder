# Proposal: phase5_go-port

## Why
Three family products already ship Go SDKs and all three hand-roll the same transport over the same msgpack library (vmihailenco/msgpack) — Go is the cheapest port with an immediate triple dedup payoff. PKG-050 slots it into the release train as the fifth lane, post-1.0 (FR-63).

## What Changes
`github.com/hivellm/thunder-go` module: wire layer on vmihailenco/msgpack v5 with UseCompactInts(true) so emitted bytes match the canonical corpus; the full SPEC-003 client contract — demux via channels, context cancellation, all 3 handshake styles, reconnect, push hook, typed errors for both prefix conventions; corpus loader in the default test run. Then the three products' Go SDKs swap internals onto thunder-go — Nexus's Go transport, Vectorizer's sdks/go/rpc, Synap's transport_rpc.go — dependency + call-site work rather than a codec rewrite, since all three already use the same msgpack lib. The module joins the release train as the fifth lane (module tag releases).

## Impact
- Governing spec: SPEC-006 (PKG-050) - docs/specs/SPEC-006-packaging-release.md; contracts per SPEC-001/SPEC-003; corpus per SPEC-005
- PRD requirements: FR-63 (P2, post-1.0)
- DAG: T5.1; depends on G5 (1.0.0 shipped)
- Affected code: go/ (new thunder-go module); Nexus Go transport, Vectorizer sdks/go/rpc, Synap transport_rpc.go (internal swaps)
- Breaking change: NO (new module; SDK swaps are internals-only, public APIs unchanged)
- User benefit: Go joins the corpus-verified language set, and three product Go SDKs stop maintaining private copies of the transport
