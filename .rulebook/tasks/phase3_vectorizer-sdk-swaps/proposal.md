# Proposal: phase3_vectorizer-sdk-swaps

## Why
Vectorizer's TS, Python, and C# SDKs each carry their own `rpc/` transport — plus the same golden hex vectors pasted per SDK, the exact drift pattern the corpus was built to end (T-012). Swapping the internals onto the Thunder packages deletes the transports while the retained golden tests double-check that Thunder produces byte-identical frames.

## What Changes
The three Vectorizer SDKs (`Vectorizer/sdks/{typescript,python,csharp}`) replace their `rpc/` transport internals with `@hivellm/thunder` / `hivellm-thunder` / `HiveLLM.Thunder` under the Vectorizer profile; the per-SDK transport sources are deleted (PKG-040). Product knowledge stays product-side per the analysis §2.3 boundary: dotted command catalogs (`search.basic`, `vectors.insert`, …), HELLO `capabilities` semantics, and the connection pools — kept as thin product-side wrappers over Thunder clients until CLT-080 lands (the SPEC-003 swap note, so the swap is never blocked on pooling). Golden vector tests are retained as the transition double-check; suites must stay green (352+ TS tests, 184+ Python tests).

## Impact
- Governing spec: SPEC-006 (PKG-040/041, PKG-021) - docs/specs/SPEC-006-packaging-release.md; SPEC-003 CLT-080 swap note
- PRD requirements: FR-62, NFR-04
- DAG: T3.5; depends on T3.1 + T3.2 + T3.3; feeds gate G3
- Affected code: Vectorizer/sdks/{typescript,python,csharp} (external repo; rpc/ transport internals deleted)
- Breaking change: NO (internals-only; public API, catalogs, capabilities, and pool surface unchanged per PKG-021; minor bumps per PKG-041)
- User benefit: one maintained codec behind the Vectorizer SDKs, golden bytes proven identical, uniform caps/timeouts/reconnect
