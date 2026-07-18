# thunder-rpc

The Rust lane of the Thunder RPC family — and the only one with a **server**.
Every HiveLLM server is Rust, so this crate carries the full stack while
TypeScript / Python / C# / Go ship clients only.

Wire bytes are identical across all five lanes: every implementation pins its
default test run to `conformance/vectors/*.yaml` (SPEC-005), so one PR changes
wire behavior everywhere or fails CI.

> **The crate is `thunder-rpc`; the library is `thunder`.** `thunder` was
> already taken on crates.io by a dormant 2018 crate, so the registry name
> differs from the import name by necessity — `cargo add thunder-rpc`, then
> `use thunder::…`.

## Install

```bash
cargo add thunder-rpc
```

```toml
# Everything (client + server) — the default.
thunder-rpc = "0.1"

# Client-only SDK: no server code compiled in.
thunder-rpc = { version = "0.1", default-features = false, features = ["client"] }

# Server only.
thunder-rpc = { version = "0.1", default-features = false, features = ["server"] }

# Pure wire layer: no tokio dependency at all.
thunder-rpc = { version = "0.1", default-features = false }

# Optional TLS transport (additive; the default build stays plaintext-only).
thunder-rpc = { version = "0.1", features = ["tls"] }
```

| Feature | Default | Pulls in | Gives you |
|---|---|---|---|
| *(none)* | — | — | `wire`: `Value`, `Request`/`Response`, frame codec, caps, `PUSH_ID` |
| `client` | ✅ | tokio | `Client`, `Pool`, endpoint parsing, typed errors |
| `server` | ✅ | tokio | `spawn_listener`, `Dispatch`, `Session`, metrics |
| `tls` | ❌ | tokio-rustls | `ClientTls` / `ServerTls` transport wrapping |

The `wire` layer never depends on tokio, so a codec-only consumer pays nothing
for the runtime (PKG-013).

## Layout

- `thunder/` — the published crate (`thunder-rpc`, lib `thunder`).
  - `wire/` — the wire layer (SPEC-001): the 8-variant `Value`, array-encoded
    `Request` / `Response`, `PUSH_ID` (= `u32::MAX`), and the length-prefixed
    MessagePack frame codec with the cap checked **before** body allocation.
  - `client/` — dial (+ optional TLS), handshake per config, a background
    reader task demultiplexing by id, connect and per-call timeouts, bounded
    in-flight, lazy reconnect, push hook, typed errors.
  - `server/` — accept loop, mpsc writer task, spawn-per-request bounded by a
    semaphore, atomic session auth, metrics.
  - `tls/` — the optional transport layer.
- `thunder-bench/` — the transport shootout harness (not published). Fourteen
  protocol lanes over one shared no-op backend; see
  [docs/analysis/protocol-shootout/](../docs/analysis/protocol-shootout/).

## Client

```rust
use thunder::{Client, ClientConfig, Config, Value};

// Identity is the application's; everything else comes from the standard.
let app = Config::standard().scheme("myapp").port(9000);

let client = Client::connect_with(
    "myapp://127.0.0.1",
    app,
    ClientConfig::new().token(jwt),
)
.await?;

let pong = client.call("PING", vec![]).await?;
let hits = client.call("SEARCH", vec![Value::Str("docs".into())]).await?;
```

Calls on one `Client` are **multiplexed**: each carries an id, replies are
matched back to their caller, and concurrent calls pipeline over a single TCP
connection rather than queueing. Clone it across tasks — no pool needed for
concurrency (`Pool` exists for a different reason: spreading load across
several connections).

## Server

Products implement one trait. Thunder owns framing, the connection state
machine, auth bookkeeping, and the quality floor.

```rust
use std::sync::Arc;
use thunder::server::{
    spawn_listener, AuthError, Credentials, Dispatch, ListenerConfig,
    Principal, ServerInfo, Session,
};
use thunder::{Config, Value};

struct Echo;

impl Dispatch for Echo {
    async fn dispatch(
        &self,
        _session: &Session,
        command: &str,
        mut args: Vec<Value>,
    ) -> Result<Value, String> {
        match command {
            "PING" => Ok(Value::Str("PONG".into())),
            "ECHO" if !args.is_empty() => Ok(args.swap_remove(0)),
            other => Err(format!("ERR unknown command '{other}'")),
        }
    }

    async fn authenticate(&self, _creds: Credentials) -> Result<Principal, AuthError> {
        Ok(Principal { name: "anonymous".into() })
    }
}

let handle = spawn_listener(
    Arc::new(Echo),
    Config::standard().scheme("myapp").port(9000),
    ServerInfo { name: "myapp".into(), version: env!("CARGO_PKG_VERSION").into() },
    ListenerConfig::default(),
)
.await?;
```

Two things worth knowing:

- **A returned `Err` never closes the connection** (SRV-005). The error string
  travels verbatim on the wire; the client raises it as a typed error and keeps
  the connection.
- **Auth is connection-sticky.** `HELLO`/`AUTH` happens once; Thunder flips the
  session flag itself, so product code never touches the state machine.

### Operating a listener

```rust
let config = ListenerConfig::default()
    // Refuse accepts past the ceiling — the socket is dropped immediately so a
    // client fails fast rather than hanging on a connection nobody will read.
    // `0` (the default) is unbounded. Bounds memory and slow-loris exposure;
    // Config::max_in_flight is a different resource (requests per connection).
    .with_max_connections(10_000)
    // Per-command callbacks: the dimensions cumulative totals cannot give.
    .with_observer(exporter);
```

**Metrics.** `handle.snapshot()` gives cumulative totals whenever you want
them. When an exporter needs **per-command labels** or **frame-size
distributions**, install a `MetricsObserver` instead of sampling: it is called
at the same point the built-in counters record — after the successful socket
write — so the two can never disagree, and there is no sampler task and no
staleness. It is `None` by default and costs nothing unset.

**Observing and shutting down at once.** `stop()` takes `&self`, and
`handle.metrics()` hands out a cheap clonable reader, so an exporter task and
graceful shutdown can coexist:

```rust
let handle = Arc::new(handle);
spawn_exporter(handle.metrics());   // reader only, no lifecycle
// …later, still graceful — drains in-flight requests:
handle.stop().await;
```

## Configuration

One standard, no per-product profiles. An application supplies its identity and
overrides only what it genuinely differs on, in its own repository:

```rust
let app = Config::standard().scheme("myapp").port(9000);

let legacy = Config::standard()
    .scheme("legacy").port(15501)
    .handshake(Handshake::AuthCommand)   // AUTH, no HELLO
    .push(PushPolicy::Enabled);          // ships a subscribe-style command
```

`scheme` and `port` have no default — identity is yours. Everything else is
pinned to [`conformance/standard.yaml`](../conformance/standard.yaml) so the
five languages cannot disagree. Full table in the
[root README](../README.md#-configuration--one-standard-zero-product-knowledge).

## Test / quality gate

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The corpus tests run by default — `cargo test` decodes
`conformance/vectors/*.yaml` and compares byte-for-byte, so a wire change
cannot pass here and fail elsewhere.

Cross-language proof that clients and servers actually talk over a socket:

```bash
python ../interop/run.py
```

## Benchmarks

```bash
cargo run -p thunder-bench --release -- --scenario all --out bench-out/
```

The harness measures its own noise floor and **refuses runs** whose qps
dispersion exceeds 5% (BEN-011) — a benchmark you cannot trust should not
produce a number. Results and honest caveats:
[docs/analysis/protocol-shootout/](../docs/analysis/protocol-shootout/).

## Upgrading to 0.2.0

Two breaking changes, both from adopting Thunder in Synap. **The wire is
untouched** — the corpus is unchanged and the cross-language interop matrix
still passes 4/4, so no other language lane and no deployed peer is affected.
Only the Rust types moved.

**1. `Value::Bytes` carries `Arc<[u8]>`, not `Vec<u8>`.**

An owned `Vec` forced a full copy of the payload in *both* directions — once
reading a value into a store, once handing a stored value to the encoder. It
scaled with payload size, so it was worst exactly where a binary protocol is
supposed to win.

```rust
// before
Value::Bytes(vec)
match v { Value::Bytes(b) => b.as_slice(), .. }

// after — construction takes anything that becomes a shared buffer
Value::bytes(vec)          // or Value::from(vec) / Value::from(arc)
match v { Value::Bytes(b) => &b[..], .. }

// and the point of the change:
let shared: Arc<[u8]> = value.into_shared_bytes().unwrap();  // refcount bump
let value = Value::bytes(Arc::clone(&stored));               // no copy
```

**2. `Dispatch` has an `Identity` associated type; `Principal` and `Session`
carry it.**

A product's resolved identity — roles, tenant, quotas — had nowhere to live,
so authorization had to re-query the credential store on every privileged
command. That was also a semantic change nobody asked for: the re-read sees
live state, so a user edited mid-session was judged by the new record.

```rust
impl Dispatch for MyServer {
    type Identity = ();          // add this line if the name is enough
    // …unchanged
}

// or carry your own, resolved once at AUTH:
impl Dispatch for MyServer {
    type Identity = User;

    async fn authenticate(&self, creds: Credentials) -> Result<Principal<User>, AuthError> {
        let user = self.store.lookup(&creds)?;          // the only lookup
        Ok(Principal::with_identity(user.name.clone(), user))
    }

    async fn dispatch(&self, session: &Session<User>, command: &str, args: Vec<Value>)
        -> Result<Value, String>
    {
        // Reads memory. No store round-trip, no String clone.
        let is_admin = session.with_principal(|p| p.is_some_and(|p| p.identity.is_admin));
        // …
    }
}
```

`Principal<I = ()>` and `Session<I = ()>` default their parameter, so the
simple case stays short. `type Identity = ();` is still required on every impl
— Rust has no stable associated-type defaults.

## Specs

| Spec | Covers |
|---|---|
| [SPEC-001](../docs/specs/SPEC-001-wire-format.md) | Wire format, `Value`, framing, caps (`WIRE-`) |
| [SPEC-002](../docs/specs/SPEC-002-configuration.md) | Config dimensions (`PRO-`) |
| [SPEC-003](../docs/specs/SPEC-003-client.md) | Client contract (`CLT-`) |
| [SPEC-004](../docs/specs/SPEC-004-server.md) | Server contract (`SRV-`) |
| [SPEC-005](../docs/specs/SPEC-005-conformance.md) | Corpus and cross-language gates (`CNF-`) |
| [SPEC-006](../docs/specs/SPEC-006-packaging-release.md) | Packaging and the release train (`PKG-`) |

## License

Apache-2.0 — same as the rest of the HiveLLM family.
