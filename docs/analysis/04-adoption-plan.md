# §4 — Adoption Plan

> Phased, gated, and ordered so that every phase ships value even if the next one slips. The sequence mirrors the one Synap itself used (binary protocol → crate extraction → SDK wiring) and the one Lexum already planned (SPEC-015 P0–P5) — Thunder slots *underneath* both.

## P0 — Decisions and scaffolding (days)

1. Freeze names: registry availability check for `thunder` (crates.io — single crate, decided 2026-07-17)/`@hivehub/thunder`/`hivellm-thunder`/`HiveLLM.Thunder`; npm org: `@hivehub` (the family org — decided 2026-07-17) (T-011/§2.5).
2. Transplant the spec (T-016) + write the profile spec (T-010's six dimensions with the Nexus/Vectorizer/Synap/Lexum columns filled in).
3. Decide the TS serialization lib (`@msgpack/msgpack` recommended, T-011).
4. Seed `conformance/vectors/` with the two Vectorizer golden vectors + the framing set (§3.1) — the corpus exists before any implementation does.

**Gate G0**: names reserved; spec + profile doc merged; corpus v0 merged.

## P1 — Rust stack + conformance harness (1–2 weeks)

1. `thunder::wire`: port from `nexus-protocol/src/rpc/` (608 LOC — the most complete of the three copies), with the canonical-`Bytes`-as-bin fix and the T-005 decode tolerances.
2. `thunder::client`: start from Vectorizer's Rust client (the only in-family Rust client with true demux), add the T-013 floor (timeouts, reconnect, error-prefix parsing, optional rustls).
3. `thunder::server`: generalize the F-010 loop + dispatch trait + metrics; profile-driven handshake (all three styles).
4. Corpus loader + full corpus + `nexus-protocol` cross-decode tests + pairwise-fuzz generator (§3.2), all in the default test run.
5. Shootout skeleton (§6.4): the shared no-op dispatch backend + Thunder and HTTP listeners, so the benchmark program grows alongside the code instead of after it.

**Gate G1**: corpus green; cross-decode green both directions; an example server (echo dispatch) serves an example client under every profile.

## P2 — Products' Rust sides swap (1–2 weeks, parallelizable per product)

Each product swap follows the four-step dissolution recipe of §5.5: server → Thunder deps (+ relocate `resp3`/`envelope` in-repo), SDK → registry deps + one-line type alias, publish the terminal `-protocol` shim, delete the crate from the workspace.

| Product | Change | Effort/risk |
|---|---|---|
| Nexus | Server + SDK onto `thunder-{wire,server,client}` with `Profile::nexus()`; `nexus-protocol` dissolved per §5 (`resp3/` moves into `nexus-server`); Rust SDK — **gains pipelining its own Rust client lacks today** (T-003) | Low — types are structurally identical; the terminal shim keeps external `nexus-protocol` consumers compiling |
| Vectorizer | Same swap + dissolution of `vectorizer-protocol`; keeps its golden tests as a double-check during the transition | Low — Thunder's client is derived from Vectorizer's |
| Synap | Same swap + dissolution of `synap-protocol` (`envelope.rs`/`resp3/` move into `synap-server`) **plus** the canonical-`Bytes` change: server starts emitting bin (already decodable by every Synap SDK, which all special-case both forms) while Thunder decodes legacy int-arrays from old SDKs | Medium — the one behavioral wire change in the whole plan; sequence server-first, verify with corpus tolerance vectors |
| Lexum | Skips its planned P1 ("create lexum-protocol") entirely: depends on `thunder` (features `server`) with `Profile::lexum()`; its SPEC-015 references Thunder's spec | Negative effort — removes a planned fourth copy (T-008) |
| Fluxum | Optional: replace `fluxum-protocol/frame.rs` with `thunder::wire`'s frame codec (frame layer only; its envelope stays its own) | Trivial, optional |

**Gate G2**: each swapped product passes its own full suite + the corpus; Synap emits bin `Bytes` and old SDKs still pass their integration tests against it; each product's Rust SDK proves `cargo publish --dry-run` with **zero path dependencies and no product-protocol package** (§5.5), and `crates/<product>-protocol` is gone from the workspace.

## P3 — TypeScript, Python, C# packages + SDK swaps (2–4 weeks, parallel per language)

1. Implement `wire + client` per §2.4, corpus-first (write the loader, then code until green).
2. Swap each product SDK's internal transport for the package, keeping public APIs and command catalogs untouched (T-018). Delete the per-SDK codec/transport files (≈11,000 LOC across the nine non-Rust transports in the four target languages).
3. This mechanically closes T-004: the nine cap-less transports and the Typeless usage cease to exist.

**Gate G3**: per language — corpus green; each product SDK's own test suite green on the swapped internals; one env-gated live smoke per product × language.

### T-018 — The swaps are dependency changes, not rewrites, because every SDK already isolates its transport

- **Evidence**: transports are internal modules everywhere — `src/transports/` (Nexus TS), `nexus_sdk/transport/`, `Transports/` (C#), `src/rpc/` (Vectorizer), `transport_rpc.py`, `internal sealed` C# classes (Synap) — with the command catalogs and public clients layered above (§1.3 sweeps).
- **Impact**: no product SDK major-version bump is required by the swap itself; users see behavior improvements (caps, timeouts, pipelining) not API changes. The one externally visible change is Synap's `Bytes` canonicalization, staged in P2.
- **Confidence**: high.

## P4 — Uniform quality floor + benchmark program (1–2 weeks)

1. Verify the T-013 floor per language with shared behavioral tests (reconnect, timeout, oversize-refusal, push routing).
2. Ship the **transport shootout** (§6.2): RESP3 + minimal-Bolt + HTTP listeners over the shared no-op engine, harness-parity clients, full scenario matrix; plus the product-level RPC-vs-HTTP harness each product runs on its real engine (seeded from Nexus's acceptance table: point read, bulk ingest, pipelined polling).
3. Confirm the terminal `-protocol` shims (published in P2, §5.4) carry `#[deprecated]` re-exports and READMEs pointing at Thunder; verify no in-family consumer still references them.

**Gate G4**: floor tests green in 4 languages; at least one product commits a benchmark artifact produced by the harness.
**Gate G5** (§6.3): Thunder wins **every cell** of the shootout matrix vs Bolt, RESP3 and HTTP (p50, p99, qps; margin ≥10%); a losing cell is a release-blocking optimization task. Quantitative public claims unlock at G5, not before.

## P5 — Fast-follows (as demanded)

- **Go port** — all three products ship Go SDKs on the same msgpack lib already (T-011); highest-value fifth language.
- **PHP/Java** — per-product until demand justifies; hand them the corpus immediately regardless (§3.1).
- **Push/streaming v-next** — when the family defines push semantics beyond Synap's SUBSCRIBE, it lands once in Thunder behind the profile.

## Effort summary

| Phase | Calendar (single engineer, familiar with the family) | Deliverable |
|---|---|---|
| P0 | 2–3 days | names, spec home, corpus v0 |
| P1 | 1–2 weeks | Rust stack + conformance harness |
| P2 | 1–2 weeks | three products swapped (Rust), Lexum unblocked |
| P3 | 2–4 weeks | TS/Py/C# packages + nine SDK swaps, ≈11k LOC deleted |
| P4 | 1–2 weeks | quality floor + transport shootout (G5) + product harnesses |

## Risk register

| Risk | Mitigation |
|---|---|
| Synap `Bytes` canonicalization breaks an unnoticed consumer | Server-first rollout; Thunder decodes both forms indefinitely until a major; corpus tolerance vectors pin the legacy form (T-005) |
| Products' release cadences resist lockstep upgrades | Products consume released semver, never git paths; re-export shims keep old crate/package names alive one deprecation cycle (T-016/T-017) |
| The module becomes a bottleneck for product-urgent fixes | Commands never touch the module (frozen v1, T-016); only wire/client behavior lives here, which is precisely what should be slow to change |
| TS lib choice proves wrong under load | Codec is behind an internal interface; the corpus + pairwise fuzz make swapping `@msgpack/msgpack` ↔ `msgpackr` a safe, measurable change (T-011) |
| A product vendor-patches its copy and drifts again | CODEOWNERS wire authority + released-versions-only consumption + corpus gates that cannot be feature-gated (T-017) |
| Enforcing caps where none existed breaks a >64 MiB user | Cap is profile-configurable (Nexus already exposes `rpc.max_frame_bytes`); Synap profile can set 512 MiB to match its crate constant until its own decision (T-005) |

### T-019 — Sequencing insight: Lexum is the forcing function

Lexum's gated plan (`Lexum/docs/analysis/hivellm-rpc/05-execution-plan.md`) has it building a fourth wire-crate copy at its P1. If Thunder P1 lands first, Lexum adopts a dependency instead — making Lexum both the first green-field consumer (validating the profile design with no legacy) and the proof that "new family project onboards by picking profile values" (T-010) holds.

### T-020 — Verdict

**Feasible, cheap relative to what it retires, and self-financing in maintenance.** The protocol is ~600 LOC frozen at v1; the family has already written it 18 times and reviewed it far more; every design decision the module needs has a production-proven answer somewhere in-family. What does not exist today — one owner, one corpus, one quality floor — is exactly what a shared module is. The recommendation is to proceed with P0 immediately.
