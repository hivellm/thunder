# Proposal: phase3_synap-sdk-swaps

## Why
Synap's SDKs carry the family's worst wire drift: a hand-rolled C# MessagePack encoder in Transport.cs (the exact thing NFR-02 forbids) and map-shaped Requests from the Python/C# SDKs instead of the canonical array encoding. Swapping the internals onto the Thunder packages retires the hand-rolled codec and canonicalizes the request shape — safely, because the server already tolerates both forms (WIRE-013).

## What Changes
The three Synap SDKs (`Synap/sdks/{typescript,python,csharp}`) replace their transport internals with `@hivehub/thunder` / `hivellm-thunder` / `HiveLLM.Thunder` under the Synap profile; transport sources are deleted (PKG-040), including the hand-rolled C# MessagePack encoder in Transport.cs. Python and C# switch from map-shaped to array-encoded Requests (WIRE-012) — no ordering constraint, since the server accepts both via the WIRE-013 tolerance. Synap's SUBSCRIBE/push flows are reimplemented over the CLT-060 push hook (dedicated-connection subscription flow per CLT-061) with user-facing semantics unchanged. Command catalogs and public APIs stay product-side and unchanged (PKG-021); suites must stay green.

## Impact
- Governing spec: SPEC-006 (PKG-040/041, PKG-021) - docs/specs/SPEC-006-packaging-release.md; SPEC-001 WIRE-012/013; SPEC-003 CLT-060/061
- PRD requirements: FR-62, NFR-04; NFR-02 (hand-rolled codec retired)
- DAG: T3.6; depends on T3.1 + T3.2 + T3.3; feeds gate G3
- Affected code: Synap/sdks/{typescript,python,csharp} (external repo; transport internals deleted incl. the C# Transport.cs encoder)
- Breaking change: NO (internals-only; wire-compatible because the server tolerates both request shapes; public API unchanged per PKG-021, minor bumps per PKG-041)
- User benefit: hand-rolled codec gone, canonical array-encoded requests, push over the standard hook, uniform caps/timeouts/reconnect
