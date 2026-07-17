# Thunder — Product Requirements Document (PRD)

| | |
|---|---|
| **Product** | Thunder — the HiveLLM binary RPC standard as a shared multi-language module (Rust, TypeScript, Python, C#; Go fast-follow) |
| **Status** | Approved for implementation (feasibility analysis complete, verdict: build) |
| **Version** | 1.0 of this document, targeting product releases 0.1.0 → 1.0.0 |
| **Date** | 2026-07-17 |
| **Owner** | Andre Ferreira |
| **Reference** | [Feasibility analysis](analysis/README.md) (findings T-001..T-026) |
| **Related** | [DAG.md](DAG.md) (implementation plan) · [SPEC index](specs/README.md) (normative specs) · [ROADMAP.md](ROADMAP.md) · [ARCHITECTURE.md](ARCHITECTURE.md) (best-of-family composite) |

---

## 1. Executive summary

Thunder is the single shared implementation of the HiveLLM binary RPC protocol — the
length-prefixed MessagePack transport that originated in Synap (SynapRPC), was specified by Nexus
(`rpc-wire-format.md`, v1 frozen) and ported byte-for-byte by Vectorizer. It ships as one package
per language (Rust wire/client/server; TypeScript, Python and C# wire/client), one language-neutral
conformance corpus, and one declarative *profile* mechanism that expresses per-product differences
(handshake, error convention, caps, push, TLS) as data instead of forks.

**The product's reason to exist is consolidation with proof.** The protocol currently exists in 18
independently maintained copies (≈17,500 LOC for a ~600-line protocol), with inconsistent feature
support, nine transports missing the mandated frame-size cap, byte-level drift already live at the
edges, and a per-product `-protocol` crate that every product is forced to publish just so its SDK
compiles. Thunder replaces all of it and proves the result: byte-compatibility becomes a CI
property (golden corpus + cross-decode + pairwise fuzz), and performance claims become committed
artifacts gated by a transport shootout against Bolt, RESP3 and HTTP that Thunder **must win in
every cell** before any claim ships.

## 2. Problem statement

The family's default SDK transport is one protocol implemented many times ([analysis §1](analysis/01-current-state.md)):

| Problem | Evidence | Impact |
|---|---|---|
| 18 copies of the wire codec (3 Rust crates + 15 SDK transports, 6 languages) | T-001 | Every fix/hardening change is re-discovered and re-implemented up to 18× |
| Feature support is scattershot (demux, reconnect, timeouts, caps, push) | T-003 | The reliability contract depends on which product × language cell a user lands in |
| 9 of 15 transports allocate from an untrusted length prefix with no cap; one uses `MessagePackSerializer.Typeless` on wire data | T-004 | Memory-exhaustion and deserialization attack surface, repeated per port |
| Byte drift is live: Synap `Bytes` as int-array (forfeits the 4× embeddings win), request map/array split, 512 vs 64 MiB caps | T-005 | Interop holds only through lenient decoders; drift compounds until it breaks |
| Conformance is per-repo; only Vectorizer pins bytes; nothing checks across languages | T-006 | "Byte-for-byte" is a comment, not a property |
| Every product must publish its `-protocol` crate to crates.io on every release | T-021 | Release choreography (protocol first, then SDK) ×3, for 95%-identical code |
| Performance claims lean on targets and conflated benchmarks | F-014/F-016/F-017, T-025 | "Beats Bolt and RESP3" is not yet citable as a transport-isolated fact |

## 3. Goals and non-goals

### Goals

1. **One implementation per language** of wire + client (plus server in Rust), owned in this repo.
2. **Byte-compatibility as CI property** — corpus, reference cross-decode, pairwise fuzz.
3. **Uniform client quality floor** in every language (pipelining, caps, timeouts, reconnect, typed errors, push hook).
4. **Zero per-product protocol packages** — servers and SDKs consume Thunder from the registries; the three existing `-protocol` crates are dissolved.
5. **Profiles, not forks** — Synap/Nexus/Vectorizer/Lexum differences expressed declaratively; new family projects onboard by picking values.
6. **Provable performance** — a transport-isolated shootout vs Bolt, RESP3 and HTTP with an always-win release gate.

### Non-goals

- **No wire v2.** Thunder implements the frozen v1 exactly. Streaming and structured error objects stay deferred, as the family already decided.
- **No command catalogs.** Product commands are opaque strings to Thunder and live in the product SDKs.
- **No replacement of HTTP/REST surfaces** — RPC remains additive in every product.
- **No PHP/Java ports at launch** (demand-driven, P5); Go is a committed fast-follow.

## 4. Users

| User | Need |
|---|---|
| Family servers (Synap, Nexus, Vectorizer, Lexum, Fluxum frame layer) | A wire + server layer they can depend on without publishing anything themselves |
| Family SDK authors (4–6 languages × 3+ products) | A transport they import instead of re-implementing; ergonomic, uniform value API |
| New HiveLLM projects | Adopt the standard by adding a dependency and picking a profile — no protocol work at all |
| Operators | One runbook posture: same caps, same metrics names, same error conventions everywhere |

## 5. Functional requirements

Priority: **P0** = required for 1.0.0 · **P1** = required for the fast-follow releases · **P2** = future.

### Wire layer ([SPEC-001](specs/SPEC-001-wire-format.md))

| ID | P | Requirement |
|---|---|---|
| FR-01 | P0 | Implement wire v1 exactly: `u32 LE length + MessagePack body`; `Request{id, command, args}` / `Response{id, result}`; 8-variant value model; rmp-serde externally-tagged encoding |
| FR-02 | P0 | Canonical encoding rules: `Bytes` emitted as MessagePack **bin**; `Request` emitted as **array**-encoded struct; compact integer forms |
| FR-03 | P0 | Legacy decode tolerances: accept int-array `Bytes` and map-shaped `Request` on decode; never emit them |
| FR-04 | P0 | Frame cap configurable (default 64 MiB), validated against the prefix **before** any allocation, on encode and decode |
| FR-05 | P0 | `PUSH_ID = u32::MAX` reserved and routed distinctly from request/response ids |
| FR-06 | P0 | The wire layer is pure (no I/O, no product knowledge) in every language |

### Profiles ([SPEC-002](specs/SPEC-002-profiles.md))

| ID | P | Requirement |
|---|---|---|
| FR-10 | P0 | A declarative `Profile` covering: handshake style, hello payload style, push policy, frame cap, in-flight bound, error convention, TLS policy |
| FR-11 | P0 | Family profile registry (synap, nexus, vectorizer, lexum) generated from data files in `conformance/profiles/`, identical across languages |
| FR-12 | P0 | Public custom-profile construction — external/new products never blocked on a Thunder release |

### Client ([SPEC-003](specs/SPEC-003-client.md))

| ID | P | Requirement |
|---|---|---|
| FR-20 | P0 | Multiplexing: demux by `id`, pipelined concurrent calls over one connection, background reader |
| FR-21 | P0 | Handshake per profile: none / optional `HELLO`+`AUTH` (Nexus style) / mandatory HELLO map with `version`, `token`/`api_key`, `client_name` (Vectorizer style) |
| FR-22 | P0 | Connect timeout (default 10 s) and per-call timeout (default 30 s); cancellation where the language has it (CancellationToken / ctx / AbortSignal) |
| FR-23 | P0 | Lazy reconnect with capped attempts and backoff; in-flight calls fail with a typed connection error |
| FR-24 | P0 | Typed errors with prefix parsing per profile: `NOAUTH`/`WRONGPASS` → auth error; `"[code] message"` → structured `code` field |
| FR-25 | P0 | Push hook: frames with `PUSH_ID` delivered to a registered handler (profile-gated); never matched to pending calls |
| FR-26 | P0 | Endpoint parsing with product scheme registration (`nexus://`, `vectorizer://`, `synap://`, bare `host:port`) |
| FR-27 | P0 | TCP_NODELAY on; bounded in-flight per connection per profile |
| FR-28 | P0 | Python ships sync **and** async clients |
| FR-29 | P1 | Optional TLS on the client (rustls / native per language) |
| FR-30 | P1 | Connection pool (N connections, round-robin), Vectorizer-pool style |

### Server — Rust only ([SPEC-004](specs/SPEC-004-server.md))

| ID | P | Requirement |
|---|---|---|
| FR-40 | P0 | Accept loop → per-connection reader + dedicated mpsc writer task → spawn-per-request bounded by semaphore |
| FR-41 | P0 | Session auth as lock-free atomic state; pre-auth allowlist and handshake enforcement per profile |
| FR-42 | P0 | Product integration via a single dispatch trait: `dispatch(session, command, args) → Result<Value, ErrorString>` |
| FR-43 | P0 | Metrics as atomics (connections, commands, errors, duration, frame bytes in/out, slow commands), snapshot-friendly |
| FR-44 | P0 | `PUSH_ID` refusal from clients; oversized frames rejected without allocation; unknown command leaves the connection usable |
| FR-45 | P1 | Optional TLS via `tokio-rustls`, config-gated (Vectorizer posture) |

### Conformance ([SPEC-005](specs/SPEC-005-conformance.md))

| ID | P | Requirement |
|---|---|---|
| FR-50 | P0 | Language-neutral golden-vector corpus (`conformance/vectors/`): frame hex + expected decoded structure; canonical, edge, framing, legacy-tolerance, push and handshake groups |
| FR-51 | P0 | A corpus loader per language; corpus green required in the default test run (no feature gates, no ignores) |
| FR-52 | P0 | Reference cross-decode against `nexus-protocol` (both directions) in Rust CI |
| FR-53 | P0 | Pairwise cross-language fuzz: random value trees encoded by each language, decoded by every other |
| FR-54 | P1 | Env-gated live interop smoke against real Synap / Nexus / Vectorizer servers on the release path |

### Packaging & release ([SPEC-006](specs/SPEC-006-packaging-release.md))

| ID | P | Requirement |
|---|---|---|
| FR-60 | P0 | Published packages: crates.io (`thunder-wire`/`thunder-client`/`thunder-server`), npm (`@hivehub/thunder`), PyPI (`hivellm-thunder`), NuGet (`HiveLLM.Thunder`) — one version per release train |
| FR-61 | P0 | Dissolution of `nexus-protocol` / `vectorizer-protocol` / `synap-protocol`: terminal deprecated re-export shims published; crates removed from product workspaces; non-RPC residue (RESP3, envelope) relocated in-repo |
| FR-62 | P0 | Product SDKs publish with zero path dependencies and no product-protocol package (`cargo publish --dry-run` proof) |
| FR-63 | P1 | Go module `github.com/hivellm/thunder-go` |

### Benchmarks ([SPEC-007](specs/SPEC-007-benchmarks.md))

| ID | P | Requirement |
|---|---|---|
| FR-70 | P0 | Transport shootout: Thunder RPC vs RESP3 vs minimal Bolt v5 vs HTTP/JSON over one shared no-op dispatch engine, harness-parity clients |
| FR-71 | P0 | Scenario matrix: 64 B echo, 4 KiB reply, 10k bulk, 768×f32 embedding, 1k pipelined, connection storm — at pipeline depths 1/16 and 1/4/16/64 connections; p50/p99/qps/bytes-on-wire per cell |
| FR-72 | P0 | Committed artifacts with environment headers; results published per release |
| FR-73 | P1 | Product-level RPC-vs-HTTP harness runnable by each family product on its real engine |

## 6. Non-functional requirements

| ID | P | Requirement |
|---|---|---|
| NFR-01 | P0 | **Wire frozen.** No byte-level change, ever, within v1; a hypothetical v2 is a new negotiated `proto` integer. Adding commands or profile fields with defaults never bumps the wire |
| NFR-02 | P0 | **Security floor.** Cap-before-allocation in every language; no `Typeless`/reflective deserialization of wire data; no hand-rolled MessagePack codecs |
| NFR-03 | P0 | **Gates cannot be bypassed.** Corpus, cross-decode and fuzz gates run in default CI and cannot be feature-gated or `#[ignore]`d |
| NFR-04 | P0 | **Non-breaking adoption.** SDK swaps must not change product SDK public APIs; the only sanctioned behavioral wire change is Synap `Bytes` canonicalization, staged server-first |
| NFR-05 | P0 | **G5 always-win.** No quantitative public claim until Thunder beats RESP3, Bolt and HTTP in every matrix cell (margin ≥ 10%); a losing cell is a release-blocking defect; every claim cites a committed artifact |
| NFR-06 | P0 | **One release train.** Products consume released registry versions only (never git paths); semver: additive = minor, tolerance removal or floor changes = major |
| NFR-07 | P0 | **Uniform floor.** The SPEC-003 client contract holds identically in all four languages, verified by shared behavioral tests |
| NFR-08 | P0 | Apache-2.0; HiveLLM family conventions (workspace lints, quality gate: type-check → lint → full suite) |
| NFR-09 | P1 | Wire layer usable in constrained contexts: no tokio dependency in `thunder-wire`; TS package works in Node ≥ 18 (browser transport out of scope for 1.0) |

## 7. Release criteria

| Release | Criteria |
|---|---|
| **0.1.0** | Rust stack (wire/client/server) + corpus + cross-decode green (gate G1) |
| **0.2.0** | Synap, Nexus, Vectorizer Rust sides swapped; `-protocol` crates dissolved; Lexum unblocked (gate G2) |
| **0.3.0** | TypeScript, Python, C# packages published; nine SDK swaps done; ≈11k LOC of duplicated transports deleted (gate G3) |
| **1.0.0** | Uniform floor verified in 4 languages (G4) **and** shootout won in every cell with committed artifacts (G5) |

## 8. Risks

Tracked with mitigations in [DAG.md](DAG.md) and [analysis §4](analysis/04-adoption-plan.md): Synap `Bytes` canonicalization (server-first, tolerances forever until major), product release-cadence coupling (registry versions + shims), TS serialization lib choice (corpus makes swapping safe), cap enforcement where none existed (profile-configurable), module-as-bottleneck (commands never touch Thunder).

## 9. References

- Canonical wire spec: `Nexus/docs/specs/rpc-wire-format.md` (v1) — transplanted to `docs/spec/` at T0.3
- Feasibility analysis: [docs/analysis/](analysis/README.md) (T-001..T-026)
- Prior adoption study: `Lexum/docs/analysis/hivellm-rpc/` (F-001..F-024)
- Committed performance evidence: `Synap/docs/benchmarks/redis-vs-synap.md`, `Nexus/bench-out/`
