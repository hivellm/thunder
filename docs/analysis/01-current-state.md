# §1 — Current State: One Protocol, Eighteen Implementations

> Evidence base: full reads of `Nexus/crates/nexus-protocol/src/rpc/`, `Vectorizer/crates/vectorizer-protocol/src/rpc_wire/`, `Synap/crates/synap-protocol/src/synap_rpc/`, and the transport layers of all 18 SDKs under `{Nexus,Synap,Vectorizer}/sdks/`. Wire-format fundamentals are specified in `Nexus/docs/specs/rpc-wire-format.md` (v1, stable) and analyzed in depth in `Lexum/docs/analysis/hivellm-rpc/01-wire-format.md` — not re-derived here.

## 1.1 The protocol in one paragraph

Frame = `u32 LE length` + MessagePack body (length counts the body only; default cap 64 MiB, checked before allocation). Body = `Request{id: u32, command: String, args: Vec<Value>}` or `Response{id: u32, result: Result<Value, String>}` in rmp-serde's externally-tagged encoding (`"Null"` bare, `{"Int": 42}` single-key map, `Result` nests: `{"Ok": {"Str": "PONG"}}`). Value enum: `Null / Bool / Int(i64) / Float(f64) / Bytes / Str / Array / Map` (Map = array of `[k, v]` pairs, keys need not be strings). Client-chosen `id`s multiplex concurrent requests over one persistent TCP connection; responses return in completion order; `u32::MAX` is reserved for server push (Synap ships it for SUBSCRIBE). Errors are strings with prefix conventions (`ERR/NOAUTH/WRONGPASS` in Nexus; `"[code] message"` in Vectorizer). v1 is frozen; adding commands never bumps the wire version.

## 1.2 Server-side inventory

| Product | Wire crate (LOC of wire layer) | RPC port | Handshake profile | Push | Frame cap | TLS |
|---|---|---|---|---|---|---|
| **Synap** (origin) | `synap-protocol/src/synap_rpc/` (495) | 15501 (HTTP 15500, RESP3 6379) | none — auth is HTTP-only | ✅ `SUBSCRIBE`, id `u32::MAX` | **512 MiB** (`codec.rs:21`) | none |
| **Nexus** (canonical spec) | `nexus-protocol/src/rpc/` (608) | 15475 (HTTP 15474, RESP3 15476) | `HELLO` optional; `AUTH [api_key]` or `[user, pass]`; pre-auth allowlist | reserved only | 64 MiB, configurable | none in 1.0 (sidecar documented) |
| **Vectorizer** (port) | `vectorizer-protocol/src/rpc_wire/` (526) | 15503 (REST 15002) | `HELLO` **mandatory first frame** — Map with `version`, `token`/`api_key`, `client_name`; reply carries `capabilities` | omitted in v1 | 64 MiB hardcoded | optional `tokio-rustls` |
| **Fluxum** (partial) | `fluxum-protocol/src/frame.rs` — frame layer explicitly labeled "HiveLLM wire standard" | — | own envelope (`[tag, payload]` messages + FluxBIN rows), **not** Request/Response | own | own | — |
| **Lexum** (pending) | none yet — SPEC-015 reserved; complete adoption plan in `Lexum/docs/analysis/hivellm-rpc/05-execution-plan.md` | 17001 planned | Vectorizer-style planned | reserved | configurable planned | reserved keys |

### T-001 — The wire codec exists in 18 independently maintained copies totalling ≈ 17,500 LOC

- **Evidence**: 3 Rust wire crates (608 + 526 + 495 LOC, near-identical — `vectorizer-protocol` says "ported byte-for-byte from Synap", `codec.rs:4-6`; `nexus-protocol` says "matches SynapValue byte-for-byte", `types.rs:6-7`) + 15 non-Rust SDK transports: Nexus TS 1,179 / Py 1,184 / C# 1,236 / Go ≈1,300 / PHP ≈1,100; Vectorizer TS 1,258 / Py 1,621 / C# 1,797 / Go 1,293; Synap TS 375 / Py 311 / C# ≈793 / Go 263 / PHP 267 / Java 657. Rust SDK clients add ≈221 (Nexus), ≈954 (Vectorizer), ≈534 (Synap) on top of their crates.
- **Impact**: every protocol clarification, bug fix or hardening change must today be discovered, re-implemented and re-tested up to 18 times. In the four target languages alone there are 12 copies. This is the entire case for the module in one number.
- **Confidence**: high (all LOC counted from source by per-repo sweeps).

## 1.3 SDK transport matrix — the four target languages

**Rust** (all three reuse their product's wire crate — the crate itself is the triplicated part):

| | Nexus (`nexus-graph-sdk` 2.5.0) | Vectorizer (`vectorizer-sdk` 3.5.0) | Synap (`synap-sdk` 1.0.0) |
|---|---|---|---|
| Demux by id | ❌ mutex single-flight, id-assert (`transport/rpc.rs:83-88`) | ✅ reader task + oneshot map (`rpc/client.rs:186-291`) | ❌ mutex single-flight (`transport/mod.rs:92-135`) |
| Reconnect | ❌ | ❌ (pool builds fresh conns) | ✅ 2-attempt |
| Timeouts | ❌ none | ❌ no connect timeout | connect only |
| Push | skip PUSH_ID | n/a | ✅ `subscribe_push` |
| Frame cap | ✅ 64 MiB (crate) | ✅ 64 MiB (crate) | ⚠️ SDK hardcodes 64 MiB vs crate's 512 MiB |

**TypeScript**:

| | Nexus `@hivehub/nexus-sdk` | Vectorizer `@hivehub/vectorizer-sdk` | Synap `@hivehub/synap` |
|---|---|---|---|
| MessagePack lib | `msgpackr` ^1.11.10 | `@msgpack/msgpack` ^3.1.3 | `msgpackr` ^2.0.4 |
| Demux by id | ✅ | ✅ | ✅ |
| Reconnect | ✅ lazy | ❌ | ✅ 2-try |
| Connect timeout | 5 s | 10 s | socket timeout |
| Frame cap | ❌ **none** (`rpc.ts:198-202`) | ✅ 64 MiB | ❌ **none** (`synap-rpc.ts:176`) |
| Golden vectors | ❌ | ✅ | ❌ |

**Python**:

| | Nexus `hivehub-nexus-sdk` | Vectorizer `vectorizer-sdk` (PyPI) | Synap `synap_sdk` |
|---|---|---|---|
| MessagePack lib | `msgpack` ≥1.0 | `msgpack` ≥1.1.1 | `msgpack` ≥1.1.0 |
| Clients | async | **sync + async** (1,621 LOC) | async |
| Demux by id | ✅ | ✅ | ✅ |
| Frame cap | ❌ **none** (`rpc.py:181-183`) | ✅ 64 MiB | ❌ **none** (`transport_rpc.py:135`) |
| Request shape | array | array | ⚠️ **map** `{"id","command","args"}` (`transport_rpc.py:184`) |
| Golden vectors | ❌ | ✅ | ❌ |

**C#**:

| | Nexus `Nexus.SDK` | Vectorizer `Vectorizer.Sdk.Rpc` | Synap `HiveLLM.Synap.SDK` |
|---|---|---|---|
| MessagePack lib | `MessagePack` 2.5.187 via **`Typeless`** ⚠️ (`Codec.cs:202,224`) | `MessagePack` 2.5.302, low-level `MessagePackWriter/Reader` (`FrameCodec.cs:102-181`) | **hand-rolled** encoder/decoder (`Transport.cs:26-451`) |
| Demux by id | ✅ | ✅ | ✅ |
| Timeouts | connect 5 s + per-request `CancellationToken` | connect 10 s + **call 30 s** | socket + linked CTS |
| Frame cap | ❌ **none** (`RpcTransport.cs:195`) | ✅ 64 MiB | ❌ **none** (`SynapRpcTransport.cs:77`) |
| Golden vectors | ❌ | ✅ | ❌ |

Other languages (out of the requested scope but relevant to planning): **Go** ships in all three products (`vmihailenco/msgpack` v5.4.1 unanimously — the only language already converged on one lib); **PHP** in Nexus + Synap (`rybakit/msgpack`); **Java** in Synap only, with a hand-rolled MessagePack codec while declaring (and never using) `jackson-dataformat-msgpack` (`SynapRpcTransport.java:340-656`).

### T-002 — The Rust wire crate is the same ~500–600 lines copy-pasted three times

- **Evidence**: §1.2 table; explicit provenance comments in both ports (cited in T-001).
- **Impact**: the cheapest, highest-leverage first step of normalization — a single `thunder`-owned crate consumed directly by the three products — already exists three times and diverges only in the type-name prefix and the frame cap (T-005). Worse, all three copies are *published* to crates.io as a forced side effect of publishing the SDKs — the release-choreography pain analyzed and resolved in §5 (T-021).
- **Confidence**: high.

### T-003 — Feature support is scattershot: no product delivers a consistent transport across its own languages, and no language is consistent across products

- **Evidence**: matrices above. Emblematic: Nexus's own *reference-language* client (Rust) cannot pipeline — it holds a mutex across write+read (`transport/rpc.rs:39,83-88`) — while its TS/Py/C#/Go clients all demux properly; Synap's Go and Java clients silently lack the push support its other five SDKs have; per-call timeouts exist only in C# and Go; **no SDK anywhere reconnects with backoff, parses Vectorizer's `[code]` error prefix, or speaks TLS on the RPC path**.
- **Impact**: users get a different reliability contract depending on which product × language cell they land in. A shared module makes the contract uniform: this is as much a product-quality fix as a deduplication.
- **Confidence**: high.

## 1.4 Security gaps

### T-004 — 9 of 15 non-Rust transports allocate from the untrusted length prefix with no cap; Nexus C# additionally deserializes wire data with `Typeless`

- **Evidence**: no frame-cap check in Nexus TS (`rpc.ts:198-202`), Py (`rpc.py:181-183`), C# (`RpcTransport.cs:195`), Go (`rpc.go:250`), PHP (`RpcTransport.php:150-157`) and Synap TS (`synap-rpc.ts:176`), Py (`transport_rpc.py:135`), C# (`SynapRpcTransport.cs:77`), PHP. All five Vectorizer SDKs enforce the cap; Synap Go/Java hardcode 64 MiB. Nexus C# uses `MessagePackSerializer.Typeless.Serialize/Deserialize` (`Codec.cs:202,224`) — the MessagePack-CSharp documentation warns Typeless embeds/loads type information and is unsafe on untrusted input; Vectorizer's low-level `MessagePackWriter/Reader` approach on the same NuGet package is the correct pattern.
- **Impact**: a malicious or buggy server can drive client memory exhaustion on 9 transports (`new byte[length]` / `readexactly(length)` from a hostile 4-byte prefix); the Typeless usage is a deserialization-attack surface. The shared module closes both classes by construction — the same way `nexus-protocol` closed it server-side (cap-before-allocation, `codec.rs:59-76`) and the same class of bug the Lexum study found in `/umicp` (F-020).
- **Confidence**: high.

## 1.5 Wire drift — it has already started

### T-005 — Three byte-level divergences exist today, all currently masked by lenient decoders

1. **`Bytes` encoding.** *(Corrected by the §7 empirical probe, T-029.)* **Every Rust implementation emits `Bytes` as a MessagePack array of integers** — plain `Vec<u8>` under rmp-serde produces the seq form (probe-verified), and Synap's `arc_bytes` adapter deliberately matches it (`synap-protocol/src/synap_rpc/types.rs:31-48`). The **bin** form appears only from some non-Rust SDKs (e.g. Vectorizer TS maps `Uint8Array` → bin, `codec.ts:48-63`, on the mistaken belief that rmp-serde defaults to bin); servers accept it only through rmp-serde's decode leniency (probe-verified both ways). The int-array form costs ~1.5 bytes per random byte vs 1.0 for bin — **no current implementation achieves the optimal binary payload** the SDK contract's `Bytes` mandate exists for (`Nexus/docs/specs/sdk-transport.md:178-181`); a 768-dim f32 embedding travels at ≈4,608 bytes instead of ≈3,077.
2. **Request shape.** Synap Python/Go/Java encode `Request` as a MessagePack **map** `{"id","command","args"}` (Java comments "server expects a MAP … NOT an array", `SynapRpcTransport.java:116-122`); Rust/TS/C#/PHP encode the rmp-serde default **array** `[id, cmd, args]`. rmp-serde deserializes structs from either, so servers tolerate both — but two "byte-identical" SDKs of the same product disagree on the bytes.
3. **Frame cap.** Synap's crate says 512 MiB (`codec.rs:21`) while its own Rust SDK hardcodes 64 MiB (`transport/mod.rs:119`); Nexus is configurable-64; Vectorizer hardcoded-64.

- **Impact**: none of this breaks interop *today*, which is exactly why it will keep accumulating until it does. The shared module must (a) canonicalize: `Bytes` = bin, `Request` = array, cap = configurable default 64 MiB; (b) tolerate the legacy forms on decode (int-array Bytes, map-shaped Request) for as long as Synap ≤1.x servers/SDKs are alive.
- **Confidence**: high.

### T-006 — Conformance is per-repo and per-language; only Vectorizer pins bytes, and nothing checks across languages or products

- **Evidence**: Vectorizer ships the golden PING frame `08 00 00 00 93 01 a4 'PING' 90` and the nested PONG response in its crate (`rpc_wire/codec.rs:206-226`, `types.rs:259-279`) and repeats the same hex in each of its 5 SDKs' unit tests. Nexus and Synap test round-trip only — a same-bug-both-sides codec change passes (F-013 made the same point server-side). No harness anywhere feeds one corpus to several implementations.
- **Impact**: the module's conformance corpus (§3) is the single highest-value *new* artifact normalization brings — today's guarantee of byte-compatibility is convention plus code comments.
- **Confidence**: high.

### T-007 — MessagePack library fragmentation is worst exactly where the module helps most

- **Evidence**: TypeScript uses three different libs/majors across the three products (§1.3); C# uses three different *strategies* (Typeless / low-level writer / hand-rolled); Java hand-rolls while declaring a dependency it never imports (`pom.xml` vs `SynapRpcTransport.java:340`). Python (`msgpack`) and Go (`vmihailenco/msgpack` v5.4.1) are already unanimous; Rust is unanimous on `rmp-serde` 1.x.
- **Impact**: fragmentation is not just aesthetic — msgpackr vs @msgpack/msgpack differ in int64/BigInt handling and extension defaults, precisely the places where the externally-tagged encoding is subtle. One lib decision per language (§2.5) retires the whole class.
- **Confidence**: high.

### T-008 — Two family projects are already queued as consumers, and one has forked rather than wait

- **Evidence**: Lexum has a complete, gated adoption plan whose P1 is "create yet another copy of the wire crate" (`Lexum/docs/analysis/hivellm-rpc/05-execution-plan.md`, P1) — a fourth copy unless the shared module lands first. Fluxum reused only the frame layer and built a divergent envelope on top (`fluxum-protocol/src/lib.rs:1-16`), which is what teams do when there is no importable standard package.
- **Impact**: the module has immediate consumers beyond the existing three (Lexum by plan, Fluxum's frame layer by construction), and delay has a concrete cost: each new project re-decides and re-implements.
- **Confidence**: high.

---

Next: [§2 — Module design](02-module-design.md) — the architecture that absorbs all of the above.
