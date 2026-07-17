## 1. Implementation
- [ ] 1.1 TS SDK: rewire onto `@hivellm/thunder` under the Vectorizer profile; delete the `rpc/` transport internals (PKG-040); dotted command catalog + capabilities semantics stay product-side (analysis §2.3), public API unchanged (PKG-021, NFR-04)
- [ ] 1.2 Python SDK: same over `hivellm-thunder`, wiring both sync and async product clients onto the matching Thunder clients (FR-28 consumers)
- [ ] 1.3 C# SDK: same over `HiveLLM.Thunder`
- [ ] 1.4 Pools: keep the existing thin product-side pool wrappers (~150 LOC pattern) over Thunder clients until CLT-080 lands - the swap is never blocked on pooling (SPEC-003 CLT-080 swap note)
- [ ] 1.5 Retain the golden vector tests as the transition double-check: Thunder-encoded frames must remain byte-identical to the pinned hex (TST-010 lineage)
- [ ] 1.6 Suites green: 352+ TS tests, 184+ Python tests, C# suite (FR-62, NFR-04)
- [ ] 1.7 Minor version bump per SDK with release notes stating the behavioral upgrades explicitly (PKG-041)

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation
- [ ] 2.2 Write tests covering the new behavior
- [ ] 2.3 Run tests and confirm they pass
