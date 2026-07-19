# hivellm-thunder

**⚡ The HiveLLM binary RPC protocol for Python — wire v1 (frozen), one configurable standard, sync + async multiplexed clients**

Thunder is the shared home of the HiveLLM binary RPC standard: a length-prefixed
MessagePack protocol (`u32 LE length` + body) multiplexing concurrent requests over one
persistent TCP connection. This package is the Python implementation — import name
`thunder_rpc`, one runtime dependency (`msgpack`), conformance-tested against the
language-neutral golden-vector corpus in [`conformance/`](../conformance/).

## Install

```bash
pip install hivellm-thunder
```

## Quickstart — sync

```python
from thunder_rpc import Client, ClientConfig, Config, Credentials, Value

# Your application's identity on top of the family standard.
config = Config.standard().with_scheme("myapp").with_port(9000)

client_config = ClientConfig(credentials=Credentials.api_key("secret-key"))
with Client.connect("myapp://localhost", config, client_config) as client:
    pong = client.call("PING")
    assert pong.as_str() == "PONG"

    hits = client.call("SEARCH", [Value.str("docs"), Value.bytes(embedding)], timeout=5.0)
```

## Quickstart — async

```python
from thunder_rpc import AsyncClient, Config

config = Config.standard().with_scheme("myapp").with_port(9000)

async with await AsyncClient.connect("myapp://localhost", config) as client:
    pong = await client.call("PING")
    assert pong.as_str() == "PONG"
```

Both clients implement the identical SPEC-003 contract: pipelined out-of-order
completion, monotonic u32 ids (skipping `PUSH_ID`), serialized writes, `max_in_flight`
backpressure, per-call timeouts (default 30 s), lazy 2-attempt reconnect with
re-handshake, typed errors, and push-frame routing. The asyncio client additionally
honors task cancellation by removing the pending entry (CLT-021).

## One standard, zero product knowledge

Thunder is a protocol library, not a product catalogue: there is no registry of named
configurations, because a library that must serve implementations which do not exist yet
cannot ship a hardcoded list of the ones that did. Instead `Config.standard()` is **the**
family standard (data, not behavior — SPEC-002), pinned to
[`conformance/standard.yaml`](../conformance/standard.yaml) by the test suite so all four
language implementations agree on what "standard" means:

| Dimension | Standard | Why |
|---|---|---|
| `handshake` | `HELLO_MANDATORY` | the only shape that negotiates `proto` and advertises capabilities |
| `hello_style` | `MAP_PAYLOAD` | `{version, token \| api_key, client_name}`; reply carries proto + capabilities |
| `push` | `RESERVED` | emitting push is a capability an application opts into |
| `max_frame_bytes` | 64 MiB | checked before allocation (WIRE-020) |
| `max_in_flight` | 256 | per-connection request bound |
| `error_codes` | `BOTH` | `"[code] message"` superset recognizing the RESP3 auth tokens — needs no negotiation |
| `tls` | `OFF` | additive capability a deployment turns on, never a dialect |

`scheme` and `default_port` are deliberately **not** part of the standard: identity is
your application's, and Thunder has no opinion about it. An application that matches the
standard writes its identity and nothing else:

```python
config = Config.standard().with_scheme("myapp").with_port(9000)
```

An application that still diverges says so in its own repository, where that knowledge
belongs — every dimension is a `with_*` override returning a new frozen `Config`
(plain `Config(...)` construction works too):

```python
config = (
    Config.standard()
    .with_scheme("legacy")
    .with_port(15501)
    .with_handshake(Handshake.AUTH_COMMAND)   # AUTH-command auth, no HELLO handler
    .with_hello_style(HelloStyle.NOT_USED)
    .with_push(PushPolicy.ENABLED)            # ships a push-producing command
)
```

Convergence is then visible and per-application: delete overrides until only identity
remains. Nobody waits on a Thunder release for a row in a registry (PRO-020).

Endpoints accept `scheme://host[:port]` — where the scheme is **your** `config.scheme` —
or bare `host:port`; `http(s)://` is rejected (Thunder is RPC-only).

## Error classes

All errors derive from `thunder_rpc.ThunderError`; branch on the class and `code`, never
on message text (CLT-052):

| Class | Meaning |
|---|---|
| `AuthError` | Handshake rejected, or `NOAUTH`/`WRONGPASS`/`NOPERM` reply |
| `ServerError` | Server answered `Err` — raw `message` + optional bracket `code` |
| `ConnectionError` | Dial/write failure, dead connection, invalid endpoint |
| `TimeoutError` | Connect or per-call timeout elapsed |
| `FrameTooLargeError` | Frame over the config's cap (checked before allocation) |
| `DecodeError` | Malformed frame, or push frame while push is `RESERVED` |

## Wire layer

`thunder_rpc.wire` is pure (no sockets): `encode_frame`, `decode_request`,
`decode_response` over the 8-variant `Value` model
(`Null | Bool | Int | Float | Bytes | Str | Array | Map`). Canonical bytes are pinned by
the corpus: `Bytes` emits as MessagePack bin, floats always pack as f64 bit-exact,
structs are array-encoded; legacy int-array `Bytes` and map-shaped `Request` decode but
are never re-emitted.

## Development

```bash
pip install -e .[dev]
python -m pytest          # includes the conformance corpus — never skipped
ruff check .
```

## Specs

- [SPEC-001 Wire format](../docs/specs/SPEC-001-wire-format.md)
- [SPEC-002 Protocol configuration](../docs/specs/SPEC-002-configuration.md)
- [SPEC-003 Client contract](../docs/specs/SPEC-003-client.md)
- [SPEC-005 Conformance](../docs/specs/SPEC-005-conformance.md)
- [SPEC-006 Packaging](../docs/specs/SPEC-006-packaging-release.md)

Apache-2.0 — part of the [Thunder](../README.md) monorepo release train.

## Typing

The package is **typed** (PEP 561): it ships a `py.typed` marker, so `mypy`,
`pyright` and friends read the annotations directly. Nothing to configure — and
if you added an `ignore_missing_imports` override for `thunder_rpc` to silence
the "missing library stubs or py.typed marker" error, you can drop it. That
override also suppressed real type mismatches against Thunder, so dropping it
is worth doing.
