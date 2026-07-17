# Proposal: phase4_behavioral-floor

## Why
Four language ports share one wire format after G3, but the corpus pins bytes, not runtime behavior — timeouts, reconnect, cap refusal and push routing could silently diverge per port. CLT-090 mandates a shared behavioral floor suite; passing it in all four languages is gate G4 and the concrete form of NFR-07 (no port ships weaker runtime guarantees than the Rust reference).

## What Changes
One scenario list, one set of assertions, executed in Rust, TypeScript, Python and C# against a scripted counterpart server: pipelined out-of-order completion; oversized inbound frame refused without allocation; connect and call timeouts firing; 2-attempt reconnect then typed failure; push routing per profile (Reserved = protocol error + poison, Enabled = hook delivery, CLT-060); error-class mapping for both conventions (NOAUTH/WRONGPASS auth prefixes per CLT-051, `"[code] "` bracket-code per CLT-050); unknown-id drop; poison-on-malformed-frame. Same scenarios, same assertions, per language — a port passes or it does not ship. Additionally the env-gated live interop smoke of TST-050 (`THUNDER_LIVE_URL_SYNAP/NEXUS/VECTORIZER`): connect, per-profile handshake, PING-class call, one typed-error call, clean close — release-path only.

## Impact
- Governing spec: SPEC-003 §9 (CLT-090) - docs/specs/SPEC-003-client.md; SPEC-005 (TST-050) for the live smoke
- PRD requirements: NFR-07
- DAG: T4.1 (gate G4); depends on G3
- Affected code: conformance/ (shared scenario definitions + scripted server behaviors); floor test suites in rust/, typescript/, python/, csharp/
- Breaking change: NO (tests only; failures surface port bugs to fix, not API changes)
- User benefit: identical failure semantics in every language — timeouts, caps, reconnect and push routing guaranteed by a suite, not by porting discipline
