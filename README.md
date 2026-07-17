# Thunder

**⚡ The HiveLLM binary RPC protocol as a single shared module — one wire, one codec, one client contract, every language**

![Status](https://img.shields.io/badge/status-0.1.0%20published-success.svg)
![Wire](https://img.shields.io/badge/wire%20protocol-v1%20(frozen)-success.svg)
![License](https://img.shields.io/badge/license-Apache--2.0-green.svg)
![Languages](https://img.shields.io/badge/targets-Rust%20%7C%20TypeScript%20%7C%20Python%20%7C%20C%23-orange.svg)

[What is Thunder?](#-what-is-thunder) • [The Protocol](#-the-protocol) • [Architecture](#-architecture) • [Packages](#-packages) • [Configuration](#-configuration--one-standard-zero-product-knowledge) • [Conformance](#-conformance) • [Benchmarks](#-benchmarks) • [Roadmap](#-roadmap) • [Documentation](#-documentation)

---

## 🎯 What is Thunder?

Thunder is the **shared home of the HiveLLM binary RPC standard** — the length-prefixed MessagePack protocol that originated in **Synap** (SynapRPC), was specified by **Nexus** (`rpc-wire-format.md` v1) and ported byte-for-byte by **Vectorizer**. It is the family's default SDK transport: persistent multiplexed TCP, no per-request handshake, no JSON tax, raw-`Bytes` embeddings.

Today that one ~600-line protocol lives in **18 independently maintained copies** (3 Rust wire crates + 15 hand-ported SDK transports across 6 languages, ≈17,500 LOC), with inconsistent feature support, 9 transports missing the mandatory frame-size cap, and byte-level drift already appearing at the edges. Thunder replaces all of them with **one implementation per language**, conformance-tested against **one golden-vector corpus**, published as **one release train**.

**Products keep what is theirs** — command catalogs, URL schemes, capability semantics. Thunder owns everything below that line: framing, the value model, the codec, the connection state machine, and the quality floor (pipelining, timeouts, reconnect, caps, error mapping).

### Why (the numbers)

| Problem today | After Thunder |
|---|---|
| 18 copies of the wire layer, ≈17.5k LOC | 4 packages (+ Go fast-follow), one owner |
| 9 of 15 SDK transports allocate from an untrusted length prefix with no cap | Cap enforced by construction, in every language |
| 3 different TypeScript msgpack libs, 3 C# strategies (incl. `Typeless` on wire data) | One vetted lib/strategy per language |
| Per-product `-protocol` crates force-published to crates.io on every release | No product publishes a protocol package again — one Thunder release train |
| Byte drift already live (Synap `Bytes` as int-array, request map/array split) | Canonical bytes emitted, legacy forms tolerated on decode, corpus-pinned |
| Golden vectors only in Vectorizer; no cross-language checks | Language-neutral corpus + pairwise fuzz + reference cross-decode in CI |

## 🧬 The Protocol

Wire format **v1 — frozen**. Adding commands never bumps the wire version.

```
┌───────────────────┬──────────────────────────┐
│  length: u32 (LE) │  body: MessagePack bytes │
└───────────────────┴──────────────────────────┘
     4 bytes              length bytes
```

- **Body**: `Request { id: u32, command: String, args: Vec<Value> }` or `Response { id: u32, result: Result<Value, String> }` — rmp-serde externally-tagged encoding (`"Null"` bare, `{"Int": 42}`, nested `{"Ok": {"Str": "PONG"}}`).
- **Value**: `Null | Bool | Int(i64) | Float(f64) | Bytes | Str | Array | Map` — `Bytes` carries raw LE-f32 embeddings with no base64 tax; `Map` is an ordered pair-list (non-string keys allowed).
- **Multiplexing**: client-chosen `id`s pipeline concurrent requests over one persistent TCP connection; responses return in completion order; `id = u32::MAX` is reserved for server push.
- **Safety**: frame cap (default 64 MiB) validated against the prefix **before** allocation.
- **Auth**: connection-sticky — `HELLO`/`AUTH` once, zero per-request overhead (the handshake shape is configurable, see [Configuration](#-configuration--one-standard-zero-product-knowledge)).

Canonical spec: `docs/spec/` (transplanted from `Nexus/docs/specs/rpc-wire-format.md` — see [§3 of the analysis](docs/analysis/03-conformance-and-versioning.md)).

## 🏗 Architecture

Thunder is a **best-of-family composite**: the server hot path comes from Synap (the only one with measured transport throughput), the client architecture from Vectorizer (the only true multiplexer), the spec and operational features from Nexus — plus upgrades none of the three has (bin `Bytes` −33% on embeddings, caps in every language, typed error-code parsing). The full donor map: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

Every HiveLLM server is Rust, so Thunder is **one full stack + three client-only ports**:

| Layer | Rust | TypeScript / Python / C# | Contents |
|---|---|---|---|
| `wire` | ✅ | ✅ | Value model, `Request`/`Response`, frame codec, caps, `PUSH_ID`. Pure functions over buffers — zero I/O, zero product knowledge. |
| `client` | ✅ | ✅ | Dial (+ optional TLS), handshake per config, background reader with demux by id, connect + per-call timeouts, bounded in-flight, lazy reconnect, push hook, typed errors with prefix parsing. |
| `server` | ✅ | — | Accept loop + mpsc writer task + spawn-per-request bounded by semaphore + atomic session auth + metrics. Products implement one trait: `dispatch(session, command, args)`. |
| `config` | ✅ | ✅ | The declarative application config (handshake style, error convention, caps, push, TLS) — one standard, every dimension a knob; see below. |

### Uniform client floor

Every Thunder client, in every language: demux by id (pipelining) · frame cap on encode **and** decode · connect timeout (10 s) + per-call timeout (30 s) · lazy reconnect with capped retries · push-frame hook · `NOAUTH`/`WRONGPASS` → typed auth errors, `"[code] message"` → structured code · TCP_NODELAY. Nothing speculative — every item is already proven somewhere in the family; Thunder makes it universal.

## 📦 Packages

All four packages are **published at 0.1.0**. Wire v1 is frozen; the API is not — 0.x may still break.

| Language | Package | Install | Serialization |
|---|---|---|---|
| Rust | `thunder-rpc` (crates.io) — one crate; `wire` always on, `client`/`server` features (default on) | `cargo add thunder-rpc` | `rmp-serde` 1.x |
| TypeScript | `@hivehub/thunder` (npm) | `npm i @hivehub/thunder` | `@msgpack/msgpack` ^3 |
| Python | `hivellm-thunder` (PyPI) — sync **and** async clients | `pip install hivellm-thunder` | `msgpack` ≥1.1 |
| C# | `HiveLLM.Thunder` (NuGet, `net8.0`) | `dotnet add package HiveLLM.Thunder` | `MessagePack` 2.5.x — low-level writer/reader only, never `Typeless` |
| Go *(fast-follow)* | `github.com/hivellm/thunder-go` | *not yet published* | `vmihailenco/msgpack` v5 |

Two registry names differ from their import names, both deliberately:

- **Rust**: `thunder` was already taken on crates.io, so the crate publishes as `thunder-rpc` while the lib name stays `thunder` — `cargo add thunder-rpc` gives you `use thunder::…`.
- **Python**: the distribution is `hivellm-thunder`, the import is `thunder_rpc` — `pip install hivellm-thunder`, then `import thunder_rpc`.

The per-product `-protocol` crates (`nexus-protocol`, `vectorizer-protocol`, `synap-protocol`) are **dissolved, not wrapped**: servers and SDKs depend on Thunder's registry packages directly; the old crates exit via a terminal deprecated re-export shim. Full recipe: [§5 of the analysis](docs/analysis/05-protocol-crate-dissolution.md).

### Usage

```rust
// Rust
use thunder::{Client, ClientConfig, Config};

let app = Config::standard().scheme("myapp").port(9000);
let client = Client::connect_with(
    "myapp://127.0.0.1",
    app,
    ClientConfig::new().token(jwt),
)
.await?;
let pong = client.call("PING", vec![]).await?;
```

```typescript
// TypeScript
import { Client, Config, Value } from "@hivehub/thunder";

const app = Config.standard().withScheme("myapp").withPort(9000);
const client = await Client.connect("myapp://127.0.0.1", app, { apiKey });
const result = await client.call("SEARCH", [Value.str("hello")]);
```

```python
# Python — sync; an AsyncClient with the same shape ships alongside it
from thunder_rpc import Client, ClientConfig, Config, Credentials, Value

app = Config.standard().with_scheme("myapp").with_port(9000)
cfg = ClientConfig(credentials=Credentials.api_key("secret-key"))

with Client.connect("myapp://localhost", app, cfg) as client:
    pong = client.call("PING")
    hits = client.call("SEARCH", [Value.str("docs")], timeout=5.0)
```

```csharp
// C#
using HiveLLM.Thunder;

var app = Config.Standard() with { Scheme = "myapp", DefaultPort = 9000 };
var cfg = new ClientConfig { Credentials = Credentials.ApiKey("secret-key") };

await using var client = await ThunderClient.ConnectAsync("myapp://localhost", app, cfg);
var pong = await client.CallAsync("PING");
```

Per-language detail: [rust](rust/) · [typescript/README.md](typescript/README.md) · [python/README.md](python/README.md) · [csharp/README.md](csharp/README.md).

## 🗂 Configuration — one standard, zero product knowledge

Thunder was born from three products' RPC implementations, but it must serve implementations that
do not exist yet. So it ships **one** configuration — the standard — and **no** named per-product
profiles. Every dimension is a knob; an application supplies its own identity and overrides only
what it actually differs on, **in its own repository**:

```rust
// An application on the standard: identity, nothing else.
let config = Config::standard().scheme("myapp").port(9000);

// One that still diverges says so — here, in its own repo.
let legacy = Config::standard()
    .scheme("legacy").port(15501)
    .handshake(Handshake::AuthCommand)   // AUTH, no HELLO
    .push(PushPolicy::Enabled);          // ships a subscribe-style command
```

**The standard** (pinned to [`conformance/standard.yaml`](conformance/standard.yaml) in all four
languages, so they can never disagree):

| Dimension | Standard | Why |
|---|---|---|
| `handshake` / `hello_style` | mandatory `HELLO` + map payload | the only shape that negotiates `proto` and advertises capabilities — what an evolving protocol needs |
| `push` | reserved | `PUSH_ID` is server→client only; *emitting* is a capability you opt into |
| `max_frame_bytes` | 64 MiB | checked before allocation |
| `max_in_flight` | 256 | per-connection bound |
| `error_codes` | `[CODE] message` superset | a strict superset needs no negotiation |
| `tls` | off | additive capability, never a dialect |

`scheme` and `port` have no default: identity is yours, and Thunder has no opinion about it.

A config fixes the handshake **shape**, never the auth **policy** — whether a deployment demands
credentials is its own config (`auth_required`). A client with no credentials simply sends no
`AUTH`, which is exactly right against an open deployment.

Convergence is then visible and per-application: delete overrides until only identity remains.

## ✅ Conformance

- **Golden-vector corpus** (`conformance/vectors/`) — language-neutral data files pairing exact frame bytes with expected decoded structure: the canonical PING/PONG pair, the full value matrix (NaN, `i64::MIN/MAX`, empty containers, non-string map keys), framing edges (partial frames, cap+1 rejection **without allocation**), and legacy-tolerance vectors (Synap int-array `Bytes`, map-shaped `Request` — decoded forever, never emitted).
- **Reference cross-decode** — every canonical frame round-trips through `nexus-protocol` in CI: Thunder is pinned to the family's existing reference, not to itself.
- **Pairwise cross-language fuzz** — random value trees encoded by each language, decoded by every other.
- **Live interop** (release path, env-gated) — smoke clients against real Synap / Nexus / Vectorizer instances.

One PR changes wire behavior in all languages at once, or fails CI. Details: [§3](docs/analysis/03-conformance-and-versioning.md).

## 📊 Benchmarks

The family already holds committed evidence: **SynapRPC ≈ 3× RESP3 and ≈ 3× Redis 7 per-op** on the same server/host (`Synap/docs/benchmarks/redis-vs-synap.md`), and **Nexus-over-RPC beats Neo4j-over-Bolt end-to-end** (250 µs vs 2,305 µs serial point read; 34.8k vs 12.1k qps at 64 workers, `Nexus/bench-out/`).

Thunder finishes the job with a **transport-isolated shootout**: Thunder RPC vs **RESP3** vs **Bolt** (minimal v5) vs **HTTP/JSON**, all over the same no-op dispatch engine, same host, harness-parity clients — measuring p50/p99/qps/bytes-on-wire across echo, bulk, embedding, pipelined and connection-storm scenarios.

**Gate G5 — always win**: no quantitative claim ships until Thunder beats every competitor in **every cell** of the matrix (margin ≥ 10%); a losing cell is a release-blocking optimization task. Design and rationale: [§6](docs/analysis/06-benchmark-mandate.md).

## 🗺 Roadmap

| Phase | Deliverable | Gate | Status |
|---|---|---|---|
| **P0** | Names reserved, spec transplanted, profile spec, corpus v0 | G0 | ⏳ next |
| **P1** | Rust `wire`+`client`+`server`, conformance harness, shootout skeleton | G1 | — |
| **P2** | Synap / Nexus / Vectorizer Rust swap; `-protocol` crates dissolved; Lexum unblocked | G2 | — |
| **P3** | TypeScript / Python / C# packages + 9 SDK swaps (≈11k LOC deleted) | G3 | — |
| **P4** | Uniform quality floor + transport shootout vs Bolt / RESP3 / HTTP | G4 · **G5** | — |
| **P5** | Go port, push/streaming v-next, PHP/Java on demand | — | — |

Full milestones: [docs/ROADMAP.md](docs/ROADMAP.md) · task graph: [docs/DAG.md](docs/DAG.md) · per-product migration and risks: [§4 of the analysis](docs/analysis/04-adoption-plan.md).

## 📚 Documentation

| Document | Contents |
|---|---|
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | The best-of-family composite — which donor contributes what, the best-of matrix, performance budget |
| [docs/PRD.md](docs/PRD.md) | Product requirements — FR/NFR catalog, release criteria |
| [docs/DAG.md](docs/DAG.md) | Implementation DAG — tasks T0.1–T5.3, gates G0–G5, critical path |
| [docs/ROADMAP.md](docs/ROADMAP.md) | Milestones M0–M5, timeline, releases 0.1.0 → 1.0.0 |
| [docs/specs/README.md](docs/specs/README.md) | Normative specs SPEC-001..007 (wire binding, profiles, client, server, conformance, packaging, benchmarks) |
| [docs/analysis/README.md](docs/analysis/README.md) | Feasibility analysis — executive summary, verdict, findings T-001..T-026 |
| [docs/analysis/01-current-state.md](docs/analysis/01-current-state.md) | The 18-implementation inventory, duplication/feature matrices, drift catalog, security gaps |
| [docs/analysis/02-module-design.md](docs/analysis/02-module-design.md) | Layered architecture, profile concept, per-language design and packaging |
| [docs/analysis/03-conformance-and-versioning.md](docs/analysis/03-conformance-and-versioning.md) | Golden-vector corpus, CI matrix, versioning and governance |
| [docs/analysis/04-adoption-plan.md](docs/analysis/04-adoption-plan.md) | Phases P0–P5, gates, effort, risks |
| [docs/analysis/05-protocol-crate-dissolution.md](docs/analysis/05-protocol-crate-dissolution.md) | Eliminating the per-product `-protocol` crates and their publishing choreography |
| [docs/analysis/06-benchmark-mandate.md](docs/analysis/06-benchmark-mandate.md) | Transport shootout vs Bolt / RESP3 / HTTP and the always-win gate |

## 🐝 HiveLLM Family

| Project | Role | Relationship to Thunder |
|---|---|---|
| [Synap](https://github.com/hivellm/synap) | KV / cache / queue server | Protocol **origin** (SynapRPC); adopts via P2, ships push today |
| [Nexus](https://github.com/hivellm/nexus) | Property graph database + vector search | Canonical **spec author**; adopts via P2 |
| [Vectorizer](https://github.com/hivellm/vectorizer) | Vector database / semantic search | Byte-for-byte **port** + golden vectors + HELLO auth; adopts via P2 |
| Lexum | Document / search engine | First **green-field consumer** — skips building its own protocol crate entirely |
| Fluxum | Streaming / table sync | Reuses the **frame layer** (own envelope); optional `thunder` wire-layer frame adoption |

## 🤝 Contributing

Wire-affecting changes require the conformance corpus, reference cross-decode and pairwise-fuzz gates green — they cannot be feature-gated or ignored. The wire format is **frozen at v1**: byte-level changes don't happen; a hypothetical v2 is a new negotiated `proto` integer, never a mutation.

## 📄 License

Apache-2.0 — same as the rest of the HiveLLM family.
