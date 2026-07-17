# Proposal: phase3_python-package

## Why
Three divergent Python transports collapse into one package, and Python is where the sync/async split is real demand, not speculation: Vectorizer's Python SDK carries both clients (1,621 LOC) while Nexus/Synap are async-only, so a shared module that ships only async would leave Vectorizer unswappable (FR-28).

## What Changes
New PyPI package `hivellm-thunder` (import name `thunder_rpc`, avoiding collision with any `thunder` package) under `python/`: wire + client only. Serialization is `msgpack` >=1.1 with `use_bin_type=True` so Bytes emits bin (WIRE-010/031). Value is a frozen dataclass `(kind, value)` + factories — the shape all three products already use. Ships BOTH a sync client (threading background reader) and an async client (asyncio), each implementing the full SPEC-003 contract with identical semantics — they differ in idiom only. Corpus loader runs in the default pytest run; ruff clean.

## Impact
- Governing spec: SPEC-001 (WIRE-001..040) - docs/specs/SPEC-001-wire-format.md; SPEC-003 (CLT-001..090) - docs/specs/SPEC-003-client.md
- PRD requirements: FR-28 (over the FR-01..FR-27 floor)
- DAG: T3.2; depends on G2; feeds T3.4–T3.6 (gate G3)
- Affected code: python/ (new package `hivellm-thunder`, import `thunder_rpc`)
- Breaking change: NO (new package; product SDKs swap onto it separately in T3.4–T3.6)
- User benefit: sync and async clients from one package with the uniform floor (caps, timeouts, reconnect, typed errors) instead of three divergent transports
