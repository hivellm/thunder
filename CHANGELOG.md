# Changelog

All notable changes to Thunder are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
semver as [SPEC-006 PKG-012](docs/specs/SPEC-006-packaging-release.md) defines
it for this project: new commands or profile fields are **minor**, decode
tolerance removal / floor changes / public API breaks are **major** — which at
`0.x` means the minor position — and canonical byte changes are **never**
(NFR-01).

**The wire format is frozen at v1.** No release on this page changes a byte on
the wire; every one is checked against the golden-vector corpus and the
cross-language interop matrix.

All packages version together (PKG-011, one release train), so a version may
appear here with no changes in a given language.

## [0.2.0] — 2026-07-18

Published: crates.io, npm, PyPI, NuGet — all four aligned. (NuGet skipped 0.1.1,
which never published; see 0.1.1 below.)

Three breaking changes, all in **Rust only**, all filed by products adopting
Thunder. The wire is untouched: the corpus passes unchanged, `Bytes` still
emits MessagePack `bin`, the legacy int-array form still decodes, and
cross-language interop still passes 4/4. The TypeScript, Python, C# and Go
lanes have no code changes in this release.

### Added

- **`wire::decode_frame_raw`** — decode a frame's body **borrowed** from the
  caller's buffer, plus the bytes consumed, with no assumption about what the
  body is. A product whose frames carry its own envelope can now reuse
  Thunder's framing (prefix, cap check, slicing) instead of reimplementing it;
  the Rust counterpart of the TypeScript `FrameReader`. `decode_frame_with_limit`
  is now this function plus a MessagePack decode, so one implementation owns
  the framing rules. ([#6], filed by Fluxum)
- **`server::MetricsObserver`** — an optional per-command callback invoked at
  exactly the point the built-in counters record, after the successful socket
  write. Gives exporters the `{command}` label and frame-size distributions
  that cumulative totals cannot reconstruct, and removes the need to sample.
  `None` by default; the command label is not even materialized unless one is
  installed. ([#3])
- **`ListenerConfig::max_connections`** — a per-listener connection ceiling.
  Accepts past the ceiling are **refused** (socket dropped immediately, so a
  client fails fast rather than hanging), and each refusal increments the new
  `MetricsSnapshot::connections_refused_total`. `0` (default) is unbounded, so
  existing deployments are unaffected. ([#2])
- **`ListenerHandle::metrics()`** — a cheap clonable metrics reader that
  carries no lifecycle, so an exporter task and graceful shutdown can coexist.
  ([#5])
- **`Value::bytes` / `as_shared_bytes` / `into_shared_bytes`** and
  `From<Arc<[u8]>>` / `From<&[u8]>` for `Value` — the zero-copy construction
  and extraction paths. ([#1])
- **`Principal::new` / `Principal::with_identity`** — construction helpers for
  the identity-carrying principal. ([#4])
- **WIRE-024** in SPEC-001: a zero-length frame is now *defined* as a valid
  keep-alive carrying no body, rather than left to each product to decide.
  ([#6])

### Changed

- **BREAKING — `Value::Bytes` carries `Arc<[u8]>` instead of `Vec<u8>`.** An
  owned `Vec` forced a full copy of the payload in **both** directions: once
  reading a value into a product's store, once handing a stored value to the
  encoder. The cost scaled with payload size, so it was worst exactly where a
  binary protocol should win — large values and the raw-LE-f32 embeddings this
  wire exists to carry. Synap had built that zero-copy path deliberately and
  lost it on adoption. **The emitted bytes are unchanged.** ([#1])
- **BREAKING — `Dispatch` gains an `Identity` associated type**, carried by
  `Principal<I>` and `Session<I>`. A product's resolved identity (roles,
  tenant, quotas) had nowhere to live, so authorization re-queried the
  credential store on every privileged command. That was also a silent
  semantics change: the re-read sees live state, so a user edited mid-session
  was judged by the new record rather than the identity captured at `AUTH`.
  Now the product chooses. Every impl must add one line, `type Identity = ();`
  — Rust has no stable associated-type defaults, so `Principal<I = ()>` and
  `Session<I = ()>` carry the ergonomics instead. ([#4])
- **BREAKING — a zero-length frame is `DecodeError::KeepAlive`**, not
  `DecodeError::Rmp`. The typed path still refuses it (a zero-length body
  genuinely cannot be a `Request`/`Response`), but callers can now match on
  intent instead of on a parse failure. `decode_frame_raw` returns it as an
  empty body. ([#6])
- **`ListenerHandle::stop` takes `&self`** instead of consuming `self`, so an
  `Arc<ListenerHandle>` can still drain gracefully. Sharing the handle used to
  make `stop()` unreachable, silently downgrading shutdown to the
  fire-and-forget `Drop` path. Existing `handle.stop().await` call sites
  compile unchanged. ([#5])

### Notes

- `thunder-bench` grew from 4 protocol lanes to **14** (Memcached, MongoDB,
  PostgreSQL, MessagePack-RPC, Thrift, gRPC, Cap'n Proto, NATS, MQTT), using
  real protocol crates wherever a Rust implementation exists. Analysis:
  [docs/analysis/protocol-shootout/](docs/analysis/protocol-shootout/). No
  shootout number is citable yet — the harness refuses runs whose noise
  exceeds 5% and no quiet host is available (BEN-031).
- AMQP and Kafka were evaluated and **deliberately not built**; the reasoning
  is recorded in
  [§4 of the analysis](docs/analysis/protocol-shootout/04-messaging-verdict.md).

## [0.1.2] — never published

Superseded by 0.2.0 before publication; its three changes are listed above
([#2], [#3], [#5]).

## [0.1.1] — 2026-07-18

Published: crates.io, npm, PyPI. **NuGet did not publish** — the API key was
rejected (403), so NuGet remains at 0.1.0.

### Fixed

- **Python**: the connect-timeout tests (sync and async) failed on
  windows-latest at `0.140 >= 0.15`. Windows' default timer granularity is
  ~15.6 ms, so a 150 ms dial can legitimately measure one tick short — the
  client waited correctly and the assertion was stricter than the clock it
  read. Both now allow one tick of slack and report the measured value on
  failure.
- **CI**: `codespell` and the Python lint lane (which runs `black`, not just
  `ruff`) were failing on pre-existing issues, surfaced by the first push to
  the default branch in a while.
- Dropped accidentally committed coverage artifacts and added them to
  `.gitignore`.

### Changed

- The npm publish job was removed from the release workflow: the `@hivehub`
  org requires an OTP, which no stored credential can provide. `@hivehub/thunder`
  is published by hand from the tagged commit; the quality gate and the
  tag-vs-manifest check still run, so a hand publish can ship neither a red
  commit nor a mismatched version. Recorded in SPEC-006 PKG-011.
- Documentation: added the missing `rust/README.md` (the only language lane
  without one, and the only one that ships a server), and brought the root
  README in line with the repository — Go promoted to a full fifth client, the
  benchmark section rewritten for 14 lanes, and the roadmap corrected.

## [0.1.0] — 2026-07-17

First release. Published: crates.io, npm, PyPI, NuGet.

### Added

- **Wire v1, frozen** (SPEC-001): `u32` LE length prefix + MessagePack body;
  the 8-variant `Value` model; array-encoded `Request`/`Response`; `PUSH_ID`
  reserved at `u32::MAX`; a frame cap validated against the prefix *before*
  allocation. Canonical `Bytes` emitted as MessagePack `bin` (−33% against the
  int-array form on embeddings), with the legacy int-array form accepted on
  decode forever.
- **Rust**: full stack — `wire` (no tokio dependency), `client` (multiplexed,
  demux by id, connect and per-call timeouts, bounded in-flight, lazy
  reconnect, push hook, typed errors), `server` (accept loop, mpsc writer task,
  spawn-per-request bounded by a semaphore, atomic session auth, metrics),
  optional `tls`.
- **TypeScript, Python, C#**: clients on the same uniform floor. Python ships
  both sync and async.
- **Go**: client, wire-identical and pinned to the same corpus.
- **One declarative configuration** with no per-product profiles: an
  application supplies its identity and overrides only what it differs on,
  pinned to `conformance/standard.yaml` in every language.
- **Conformance**: language-neutral golden-vector corpus, reference
  cross-decode against `nexus-protocol`, pairwise cross-language fuzz, and a
  live interop matrix where every client dials a real Rust server over a real
  socket.

[#1]: https://github.com/hivellm/thunder/issues/1
[#2]: https://github.com/hivellm/thunder/issues/2
[#3]: https://github.com/hivellm/thunder/issues/3
[#4]: https://github.com/hivellm/thunder/issues/4
[#5]: https://github.com/hivellm/thunder/issues/5
[#6]: https://github.com/hivellm/thunder/issues/6
