# @hivehub/thunder

**⚡ HiveLLM binary RPC for TypeScript — the family wire (v1, frozen) + a multiplexed, profile-driven client**

One frame is `u32 LE length` + MessagePack body over the 8-variant value model
(`Null | Bool | Int(i64) | Float(f64) | Bytes | Str | Array | Map`). This package is the
TypeScript lane of [Thunder](https://github.com/hivellm/thunder): byte-compatible with the
Rust, Python and C# packages, pinned to the same conformance corpus in CI.

## Install

```sh
npm install @hivehub/thunder
```

Node ≥ 18. Sole runtime dependency: `@msgpack/msgpack`. Dual ESM + CJS build.

## Quickstart

```ts
import { Client, Profiles, Value } from "@hivehub/thunder";

const client = await Client.connect("vectorizer://localhost", Profiles.vectorizer, {
  credentials: { type: "apiKey", apiKey: "secret" },
  clientName: "my-app",
});

const pong = await client.call("PING");
console.log(Value.asStr(pong)); // "PONG"

// Concurrent calls pipeline over the one connection and complete in
// server order (CLT-010); per-call timeout and AbortSignal supported.
const results = await Promise.all([
  client.call("SEARCH", [Value.str("docs"), Value.str("query")]),
  client.call("SEARCH", [Value.str("docs"), Value.str("other")], { timeoutMs: 5_000 }),
]);

await client.close();
```

Endpoints accept `scheme://host[:port]` for every registered profile scheme (the port
defaults from the registry) plus bare `host:port`. `http(s)://` is rejected — Thunder is
RPC-only; REST belongs to the product's HTTP client.

The client floor, uniform across all Thunder languages: TCP_NODELAY, connect timeout
10 s, per-call timeout 30 s, demux by id with pipelining, `maxInFlight` backpressure,
lazy reconnect (2 attempts, capped backoff, no silent replay), push-frame hook, frame
cap validated against the length prefix **before** allocation.

## Profiles

Product differences are data, not forks ([SPEC-002](../docs/specs/SPEC-002-profiles.md)):

| Profile | Scheme / port | Handshake | Error convention | Push |
|---|---|---|---|---|
| `Profiles.synap` | `synap://` 15501 | `AUTH` (no `HELLO`) | RESP3 prefixes | enabled |
| `Profiles.nexus` | `nexus://` 15475 | arg-less `HELLO` optional + `AUTH` | RESP3 prefixes | reserved |
| `Profiles.vectorizer` | `vectorizer://` 15503 | `HELLO` mandatory | `"[code] message"` | reserved |
| `Profiles.lexum` | `lexum://` 17001 | `HELLO` mandatory | both | reserved |

Custom profiles never wait for a Thunder release: spread a constant —
`{ ...Profiles.vectorizer, name: "acme", scheme: "acme", defaultPort: 9000 }`.

## Errors

Every failure is a `ThunderError` with a stable `errorClass` — branch on the class (or
the subclass) and `code`, never on message text:

| Class | Subclass | Meaning |
|---|---|---|
| `auth` | `AuthError` | Handshake rejection, `NOAUTH`/`WRONGPASS`/`NOPERM` replies |
| `server` | `ServerError` | Server `Err` reply; `code` carries a parsed `"[code] "` prefix |
| `connection` | `ConnectionError` | Dial/write failure, connection died, client closed |
| `timeout` | `TimeoutError` | Connect or per-call timeout elapsed |
| `frame-too-large` | `FrameTooLargeError` | Inbound frame past the profile cap (connection poisoned) |
| `decode` | `DecodeError` | Malformed frame, or push under a `reserved` profile (poisoned) |

## Wire layer

The codec is pure (no sockets) and exported for server-side or advanced use:
`encodeRequest` / `encodeResponse`, `decodeRequest` / `decodeResponse` (one frame +
bytes consumed, `null` = need more bytes), `decodeRequestBody` / `decodeResponseBody`,
and the streaming `FrameReader`. `Value.int` is a `bigint` (full i64 range; plain
numbers accepted within the safe-integer range), `Value.bytes` is a `Uint8Array`
emitted as MessagePack `bin` — the legacy int-array form decodes forever, and is never
emitted.

## Conformance

`npm test` always runs the language-neutral golden-vector corpus from
[`conformance/vectors/`](../conformance) — 38 vectors covering canonical bytes, the
full value matrix (NaN bit patterns, i64 extremes, empty containers, non-string map
keys), framing edges (cap+1 rejection without allocation, partial input, streams) and
the frozen legacy tolerances — plus a behavioral suite against an in-process mock
server mirroring the Rust reference client tests.

## Specs

Normative contracts: [SPEC-001 wire format](../docs/specs/SPEC-001-wire-format.md) ·
[SPEC-002 profiles](../docs/specs/SPEC-002-profiles.md) ·
[SPEC-003 client](../docs/specs/SPEC-003-client.md) ·
[SPEC-005 conformance](../docs/specs/SPEC-005-conformance.md) ·
[SPEC-006 packaging](../docs/specs/SPEC-006-packaging-release.md)

## License

Apache-2.0 — same as the rest of the HiveLLM family.
