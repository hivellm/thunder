# hivellm-thunder

**⚡ The HiveLLM binary RPC protocol for Python — wire v1 (frozen), family profiles, sync + async multiplexed clients**

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
from thunder_rpc import Client, ClientConfig, Credentials, Profiles, Value

config = ClientConfig(credentials=Credentials.api_key("secret-key"))
with Client.connect("vectorizer://localhost", Profiles.vectorizer, config) as client:
    pong = client.call("PING")
    assert pong.as_str() == "PONG"

    hits = client.call("SEARCH", [Value.str("docs"), Value.bytes(embedding)], timeout=5.0)
```

## Quickstart — async

```python
from thunder_rpc import AsyncClient, Profiles

async with await AsyncClient.connect("nexus://localhost", Profiles.nexus) as client:
    pong = await client.call("PING")
    assert pong.as_str() == "PONG"
```

Both clients implement the identical SPEC-003 contract: pipelined out-of-order
completion, monotonic u32 ids (skipping `PUSH_ID`), serialized writes, `max_in_flight`
backpressure, per-call timeouts (default 30 s), lazy 2-attempt reconnect with
re-handshake, typed errors, and push-frame routing. The asyncio client additionally
honors task cancellation by removing the pending entry (CLT-021).

## Family profiles

Profiles are data, not behavior (SPEC-002) — constants pinned to
[`conformance/profiles/*.yaml`](../conformance/profiles/) by the test suite:

| Profile | Endpoint scheme | Port | Handshake | Push | Error convention |
|---|---|---|---|---|---|
| `Profiles.synap` | `synap://` | 15501 | `AUTH` (no `HELLO`) | enabled | RESP3 prefixes |
| `Profiles.nexus` | `nexus://` | 15475 | arg-less `HELLO` optional + `AUTH` | reserved | RESP3 prefixes |
| `Profiles.vectorizer` | `vectorizer://` | 15503 | `HELLO` mandatory | reserved | `[code] message` |
| `Profiles.lexum` | `lexum://` | 17001 | `HELLO` mandatory | reserved | both |

Custom profiles are plain `Profile(...)` construction — a new product never waits for a
Thunder release (PRO-020). Endpoints accept `scheme://host[:port]` or bare `host:port`;
`http(s)://` is rejected (Thunder is RPC-only).

## Error classes

All errors derive from `thunder_rpc.ThunderError`; branch on the class and `code`, never
on message text (CLT-052):

| Class | Meaning |
|---|---|
| `AuthError` | Handshake rejected, or `NOAUTH`/`WRONGPASS`/`NOPERM` reply |
| `ServerError` | Server answered `Err` — raw `message` + optional bracket `code` |
| `ConnectionError` | Dial/write failure, dead connection, invalid endpoint |
| `TimeoutError` | Connect or per-call timeout elapsed |
| `FrameTooLargeError` | Frame over the profile cap (checked before allocation) |
| `DecodeError` | Malformed frame, or push frame under a `reserved` profile |

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
- [SPEC-002 Profiles](../docs/specs/SPEC-002-profiles.md)
- [SPEC-003 Client contract](../docs/specs/SPEC-003-client.md)
- [SPEC-005 Conformance](../docs/specs/SPEC-005-conformance.md)
- [SPEC-006 Packaging](../docs/specs/SPEC-006-packaging-release.md)

Apache-2.0 — part of the [Thunder](../README.md) monorepo release train.
