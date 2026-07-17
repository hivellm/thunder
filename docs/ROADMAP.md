# Thunder — Roadmap

> Current phase: **Design complete** — feasibility analysis, PRD, specs and DAG done; implementation
> starting at M0. Milestones map 1:1 to the [DAG](DAG.md) phases; each gate (`G<n>`) is the
> milestone's exit criterion. Four releases are planned: **0.1.0** (Rust stack), **0.2.0** (family
> Rust swap + `-protocol` dissolution), **0.3.0** (four languages), **1.0.0** (quality floor +
> shootout won). The launch bar is set by the [PRD](PRD.md): uniform client floor in four languages
> (NFR-07), zero per-product protocol packages (FR-61/62), and the always-win benchmark gate
> (NFR-05/G5).

Status legend: ✅ Completed · 🚧 In Progress · 📋 Planned · 🔮 Future (post-launch)

---

## Timeline

```
2026 Q3                                          2026 Q4
│                                                │
├─ M0 Bootstrap & decisions (G0)  ~1 wk          │
│   └─ M1 Rust stack + conformance (G1) ~2 wk    │
│        └─ M2 Family swap + dissolution (G2) ~2 wk (products in parallel)
│             └─ M3 TS / Python / C# (G3) ~3-4 wk (languages in parallel)
│                  └─ M4 Floor + shootout (G4, G5) ~2 wk
│                       └─ 1.0.0 ────────────────┤
│                                                └─ M5 Go port · push v-next 🔮
```

Estimates assume one engineer familiar with the family; Phase 2 and Phase 3 shrink with parallel
hands (product swaps and language ports are mutually independent — see [DAG §3](DAG.md#3-critical-path)).

---

## Milestones

### M0 — Bootstrap & decisions (DAG Phase 0) 📋
**Gate:** G0 | **Blocks:** everything | **Tasks:** T0.1–T0.4

Monorepo skeleton (`rust/`, `typescript/`, `python/`, `csharp/`, `conformance/`) with the family
CI posture (fmt + clippy `-D warnings` + tests ×3 OS; per-language lint/test lanes); registry name
reservations (crates.io `thunder-*`, npm org decision, PyPI `hivellm-thunder`, NuGet
`HiveLLM.Thunder`); wire-spec transplant into `docs/spec/` with provenance; profile dimensions
specified; corpus v0 (canonical PING/PONG + framing vectors) loadable.

**Definition of done:** names reserved; spec + profile spec merged; corpus v0 parses in CI.

---

### M1 — Rust stack + conformance harness (DAG Phase 1) 📋
**Gate:** G1 | **Depends on:** M0 | **Tasks:** T1.1–T1.6 | **Release:** **0.1.0**

| Component | Description |
|---|---|
| `thunder-wire` | Value model + `Request`/`Response` + frame codec; canonical `Bytes`=bin, array `Request`, legacy decode tolerances; configurable cap, checked pre-allocation ([SPEC-001](specs/SPEC-001-wire-format.md)) |
| Conformance | Full corpus + Rust loader; cross-decode vs `nexus-protocol` both ways; pairwise-fuzz generator ([SPEC-005](specs/SPEC-005-conformance.md)) |
| `thunder-client` | Demux by id, three handshake styles, connect/call timeouts, reconnect, push hook, error-prefix parsing, endpoint parser, optional rustls ([SPEC-003](specs/SPEC-003-client.md)) |
| `thunder-server` | Accept loop + writer task + semaphore + dispatch trait + metrics + profile enforcement ([SPEC-004](specs/SPEC-004-server.md)) |
| `thunder-bench` skeleton | No-op dispatch backend + Thunder & HTTP listeners ([SPEC-007](specs/SPEC-007-benchmarks.md)) |

**Definition of done:** corpus + cross-decode green in the default test run (no feature gates);
example echo server ⇄ client under every registered profile; public API shape frozen.

---

### M2 — Family swap + `-protocol` dissolution (DAG Phase 2) 📋
**Gate:** G2 | **Depends on:** M1 | **Tasks:** T2.1–T2.5 | **Release:** **0.2.0**

Synap, Nexus and Vectorizer servers + Rust SDKs move onto Thunder (parallel per product). The
three `-protocol` crates are dissolved per [SPEC-006](specs/SPEC-006-packaging-release.md): non-RPC
residue (RESP3, HTTP envelope) relocates into the servers, terminal `#[deprecated]` re-export
shims are published, crates leave the workspaces. Synap canonicalizes `Bytes` to bin,
server-first. Lexum onboards green-field with `Profile::lexum()` instead of building
`lexum-protocol`.

**Definition of done:** product suites + corpus green; Synap emits bin and existing SDKs still
pass; every product Rust SDK proves `cargo publish --dry-run` with zero path deps and no protocol
package.

---

### M3 — TypeScript, Python, C# (DAG Phase 3) 📋
**Gate:** G3 | **Depends on:** M2 | **Tasks:** T3.1–T3.6 | **Release:** **0.3.0**

| Package | Notes |
|---|---|
| `@hivellm/thunder` | `@msgpack/msgpack`, streaming FrameReader with cap, `bigint` Int policy, ESM+CJS |
| `hivellm-thunder` | sync **and** async clients, `msgpack` ≥1.1 |
| `HiveLLM.Thunder` | low-level `MessagePackWriter`/`Reader` (never `Typeless`), per-call `CancellationToken` |

Then the nine product SDK internals (3 products × 3 languages) swap to the packages — public APIs
unchanged, per-SDK codec/transport files deleted (≈11k LOC). This mechanically closes the
frame-cap and `Typeless` security gaps (analysis T-004).

**Definition of done:** corpus green in all four languages; pairwise fuzz green; the nine SDK test
suites green on swapped internals; one env-gated live smoke per product × language.

---

### M4 — Quality floor + transport shootout (DAG Phase 4) 📋
**Gates:** G4 · **G5** | **Depends on:** M3 | **Tasks:** T4.1–T4.4 | **Release:** **1.0.0**

Shared behavioral floor suite (reconnect, timeouts, oversize refusal, push routing) executed in
all four languages (G4). The transport shootout completes: RESP3 + minimal Bolt v5 listeners over
the shared no-op backend, parity clients validated against `redis-benchmark`, full scenario matrix
(echo / 4 KiB / bulk 10k / 768×f32 embedding / 1k pipelined / connection storm × depths × 1–64
connections), artifacts committed with environment headers ([SPEC-007](specs/SPEC-007-benchmarks.md)).

**Definition of done (G5 — the always-win gate):** Thunder beats RESP3, Bolt and HTTP on p50, p99
and qps in **every cell** (margin ≥ 10%); a losing cell blocks release and becomes an optimization
task. Quantitative public claims unlock here, and nowhere earlier.

---

### M5 — Fast-follows 🔮
**Depends on:** 1.0.0 | **Tasks:** T5.1–T5.3

- **Go port** (`thunder-go`) — all three products already ship Go SDKs on the same msgpack lib; highest-value fifth language.
- **Push/streaming v-next** — family push semantics beyond Synap's SUBSCRIBE, wire-compatible via the reserved `PUSH_ID`; coordinated with Synap before frame semantics are defined.
- **PHP / Java** — demand-driven; both receive the conformance corpus immediately regardless.

---

## Out of scope (recorded so nobody wonders)

- **Wire v2 / streaming frames** — deferred family-wide; v1 is frozen (PRD NFR-01).
- **Browser transport for TypeScript** — Node ≥ 18 only at 1.0 (PRD NFR-09).
- **RESP3/Bolt as Thunder features** — they exist in this repo only as shootout competitors (SPEC-007); Thunder ships one protocol.
