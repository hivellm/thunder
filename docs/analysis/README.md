# HiveLLM Binary RPC → Shared Multi-Language Module — Feasibility Analysis

> **Question**: Can the HiveLLM binary RPC standard (length-prefixed MessagePack, born in Synap, spec'd by Nexus, ported by Vectorizer) become a single shared module for **Rust, TypeScript, Python and C#**, so every application in the family normalizes on one implementation?
>
> **Answer**: **Yes — and the evidence says it is overdue.** The wire codec currently exists in **18 independently maintained copies** (3 Rust crates + 15 hand-ported SDK transports across 6 languages), totalling ≈ **17,500 LOC** that implement the same ~600-line protocol. The copies have already drifted (encoding of `Bytes`, request shape, frame caps, auth handshakes), 9 of the 15 SDK transports skip the frame-size cap the spec mandates, and no cross-language conformance harness exists. The protocol itself is deliberately tiny and frozen at v1 — which is exactly what makes normalization cheap. The hard part is governance and migration, not code.
>
> **Research date**: 2026-07-17. Sources read: the canonical spec (`Nexus/docs/specs/rpc-wire-format.md` v1), the three protocol crates (`nexus-protocol`, `vectorizer-protocol`, `synap-protocol`), all 18 SDK transport implementations across the three products, Fluxum's partial adoption (`fluxum-protocol`), and the prior adoption study `Lexum/docs/analysis/hivellm-rpc/` (findings F-001..F-024, reused here where relevant).

## Section index

| § | File | Contents |
|---|---|---|
| §1 | [01-current-state.md](01-current-state.md) | Protocol recap, full implementation inventory, duplication and feature matrices, wire-drift catalog, security gaps (T-001..T-008) |
| §2 | [02-module-design.md](02-module-design.md) | Proposed module architecture: wire/client/server layers, the protocol-profile concept, per-language design and packaging (T-009..T-014) |
| §3 | [03-conformance-and-versioning.md](03-conformance-and-versioning.md) | Language-neutral golden-vector corpus, cross-implementation CI, versioning and governance rules (T-015..T-017) |
| §4 | [04-adoption-plan.md](04-adoption-plan.md) | Phased migration per product, effort estimates, risk register (T-018..T-020) |
| §5 | [05-protocol-crate-dissolution.md](05-protocol-crate-dissolution.md) | Eliminating the per-product `-protocol` crates and their forced publishing choreography (T-021..T-024) |
| §6 | [06-benchmark-mandate.md](06-benchmark-mandate.md) | Transport shootout Thunder vs Bolt vs RESP3 vs HTTP; the always-win release gate G5 (T-025..T-026) |
| §7 | [07-performance-baseline.md](07-performance-baseline.md) | Which of the three implementations is fastest — hot-path comparison, rmp-serde probe, the composite baseline (T-027..T-030) |

Findings are numbered **T-001..T-030** globally.

## Executive summary

**The standard is one protocol implemented many times.** A frame is `u32 LE length + MessagePack body`; the body is `Request{id, command, args}` / `Response{id, result}` over an 8-variant value enum (`Null/Bool/Int/Float/Bytes/Str/Array/Map`) in rmp-serde's externally-tagged encoding; requests multiplex out-of-order over one persistent TCP connection demultiplexed by `id`; `u32::MAX` is reserved for server push; a 64 MiB cap is checked before allocation; v1 is frozen — new commands never bump the wire version. Three server products (Synap origin, Nexus canonical spec, Vectorizer byte-for-byte port) and 18 SDK transports speak it today; Fluxum reuses the frame layer with its own envelope; Lexum has a complete adoption plan pending (SPEC-015).

**Duplication is real and already leaking.** Per target language: Rust has 3 near-identical wire crates (608/526/495 LOC); TypeScript has 3 transports on 3 different MessagePack libraries (`msgpackr` 1.x, `msgpackr` 2.x, `@msgpack/msgpack` 3.x); Python has 3; C# has 3 — one on `MessagePackSerializer.Typeless` (unsafe on untrusted input), one on the safe low-level writer, one entirely hand-rolled. Feature support is inconsistent everywhere: true multiplexing exists in 10 of 15 non-Rust transports but in neither product's *Rust* client except Vectorizer's; reconnect, timeouts, TLS and error-code parsing vary per port with no pattern. Only Vectorizer ships golden byte vectors.

**The divergences are still small enough to unify.** All observed drift (Synap's `Bytes` as int-array vs bin, request struct as array vs map, 512 MiB vs 64 MiB caps, three auth handshake styles, two error-prefix conventions) is tolerated by rmp-serde's decoder or is a per-product *profile* decision, not a wire fork. A shared module canonicalizes the bytes, tolerates the legacy forms on decode, and expresses the product differences as a small profile config — exactly the six-dimension divergence table the Lexum study identified (F-011).

**Proposed shape** (§2): a monorepo (this repository — Thunder) hosting four packages plus a language-neutral conformance corpus. Every family server is Rust, so only Rust needs a server layer; TypeScript/Python/C# are client + codec only. Products keep what is genuinely theirs: command catalogs, URL schemes/ports, capability semantics. Migration (§4) is a dependency swap: SDK internals swap without breaking public APIs.

**The `-protocol` crates dissolve — products stop publishing protocol packages** (§5). Today every product must publish its protocol crate to crates.io just so its SDK can compile (`vectorizer-protocol` "published first" choreography, `nexus-protocol` 2.5.0 pinned by the SDK), and those crates even drag server-internal code (RESP3, HTTP envelope) onto a public registry. End state: servers and SDKs depend on Thunder's registry packages directly; the per-product protocol footprint shrinks to a ~10-line `Profile` shipped inside Thunder's family registry; already-published crates exit via a terminal re-export shim, then leave the workspace.

### Top findings by impact

| # | Impact | Finding |
|---|---|---|
| T-001 | Case for the module | 18 copies of one ~600-LOC protocol, ≈17.5k LOC total; 12 copies in the four target languages alone |
| T-004 | Security | 9 of 15 SDK transports allocate from the untrusted length prefix with no cap; Nexus C# deserializes wire data with `MessagePackSerializer.Typeless` |
| T-005 | Interop risk | Wire drift exists **today**: every Rust implementation emits `Bytes` as an int-array (~50% bigger than bin on embeddings — §7 probe), some non-Rust SDKs emit bin, and Python/Go/Java Synap SDKs send requests as maps while others send arrays |
| T-003 | Quality inconsistency | Feature matrix is scattershot: demux, reconnect, timeouts, caps and push support differ per product × language with no pattern |
| T-009 | Design | All family servers are Rust → the module is 1 full stack (Rust: wire+client+server) + 3 client-only ports (TS/Py/C#) — much smaller than it sounds |
| T-010 | Design | The six known divergence dimensions (naming, handshake, push, caps, error prefix, TLS) become a declarative `Profile`, so one module serves Synap-, Nexus- and Vectorizer-shaped servers |
| T-015 | Conformance | A single language-neutral golden-vector corpus (hex frames + expected values), consumed by all four test suites and cross-checked against `nexus-protocol`, replaces today's per-repo, per-language re-assertions |
| T-018 | Migration | Adoption is a dependency swap, not a rewrite: transports are internal modules in every SDK; public APIs don't change |
| T-021 | Release pain | Publishing an SDK forces publishing the product's `-protocol` crate — a per-product release choreography for 95%-identical code |
| T-022 | Release fix | Thunder inverts the dependency: one release train replaces three protocol-then-SDK choreographies; no product publishes a protocol package again |
| T-025 | Performance | Committed evidence upgraded: Synap's artifact shows SynapRPC ~3× RESP3/Redis per-op, transport-isolated; Nexus beats Neo4j-over-Bolt end-to-end |
| T-026 | Performance gate | G5: Thunder must win every cell of the Bolt/RESP3/HTTP shootout matrix before any quantitative claim ships |
| T-027 | Baseline choice | Synap's listener is the most performance-engineered server (BufWriter coalescing +23%, nodelay, zero-copy replies); Nexus's pays 2 extra serializations per op |
| T-029 | Baseline choice | Probe-verified: all three Rust implementations emit int-array `Bytes`; Thunder's canonical bin is ~33% smaller on embeddings and decodes on every deployed server |

### Performance positioning (§6)

The direction is already established by committed artifacts: Synap holds a **transport-isolated** same-server comparison where native SynapRPC is **~3× RESP3 and ~3× Redis 7 per-op** (`Synap/docs/benchmarks/redis-vs-synap.md` — this partially supersedes the Lexum study's F-017, which was Nexus-scoped), and Nexus holds committed **end-to-end** wins over Neo4j-over-Bolt (250 µs vs 2,305 µs serial point read; 34.8k vs 12.1k qps at 64 workers), engine and transport conflated (F-016). What is missing is one matrix, one host, one methodology for all competitors at once — with harness-parity clients (Synap's own artifact documents how a weak bench client understated the native path at pipeline depth 16). §6 designs that shootout — Thunder vs Bolt vs RESP3 vs HTTP over a shared no-op engine — and gate **G5: Thunder must win every cell** before any quantitative claim ships; a losing cell is a release-blocking optimization task.

### Recommendation

Build it, in this repository, in four phases (§4): **P0** name/scope decisions + spec transplant → **P1** Rust `wire`+`client`+`server` with the conformance corpus → **P2** the three products' Rust sides swap to it and the `-protocol` crates are dissolved per §5 (terminal shim published, crate deleted from the workspace) → **P3** TypeScript/Python/C# packages + SDK swaps, closing the cap/timeout/reconnect gaps as a side effect. Go is a recommended fast-follow (all three products ship a Go SDK); PHP/Java remain per-product until demand justifies them.

### Related analyses

- [behavioral-normalization/](behavioral-normalization/README.md) — the sequel question (2026-07-17):
  can the per-product behavioral differences the `Profile` parameterizes (handshake, errors, push,
  caps, TLS) be **eliminated** so all four products speak exactly the same way? Verdict: yes — four
  of five dimensions at near-zero cost, handshake via a dual-accept migration (BN-001..BN-023). Its
  source sweep also exposed three profile-registry errata (BN-023) that correct assumptions made
  here (notably: Synap's RPC path *does* authenticate behind `require_auth`, contra §1's
  "auth is HTTP-only" reading).
- [push-streaming/](push-streaming/README.md) — **proposal-stage** design (T5.2, P2 fast-follow):
  family push/streaming semantics beyond Synap's `SUBSCRIBE` (watch, progress, invalidation),
  wire-compatible via the reserved `PUSH_ID` so the wire version stays `1`. Proposes a canonical
  push envelope `{stream, kind, data}` inside the existing `Ok(Value)` payload and a
  backward-compatible `push = Streaming` profile setting (PUSH-001..PUSH-062). Proposal-stage corpus
  vectors live under [`conformance/vectors/proposal-push-streaming/`](../../conformance/vectors/proposal-push-streaming/README.md)
  and are verified to round-trip the frozen wire; ratification is gated on Synap coordination.
