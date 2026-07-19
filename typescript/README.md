# @hivehub/thunder

**⚡ HiveLLM binary RPC for TypeScript — the family wire (v1, frozen) + a multiplexed, config-driven client**

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
import { Client, Config, Value } from "@hivehub/thunder";

// Your application's identity; every behavior is the standard.
const config = Config.standard().withScheme("myapp").withPort(9000);

const client = await Client.connect("myapp://localhost", config, {
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

Endpoints accept `scheme://host[:port]` — the scheme being your config's own, resolving
your config's `defaultPort` — plus bare `host:port`. `http(s)://` is rejected — Thunder is
RPC-only; REST belongs to your application's HTTP client.

The client floor, uniform across all Thunder languages: TCP_NODELAY, connect timeout
10 s, per-call timeout 30 s, demux by id with pipelining, `maxInFlight` backpressure,
lazy reconnect (2 attempts, capped backoff, no silent replay), push-frame hook, frame
cap validated against the length prefix **before** allocation.

## Configuration

Thunder ships **one standard and zero product knowledge**
([SPEC-002](../docs/specs/SPEC-002-configuration.md)). There are no named configurations: a
protocol library that must serve implementations which do not exist yet cannot ship a
hardcoded list of the ones that did. `Config.standard()` is **the** family standard, and
every dimension is a knob:

| Dimension | Standard | Meaning |
|---|---|---|
| `scheme` / `defaultPort` | `""` / `0` | **Identity — yours.** Thunder has no default |
| `handshake` | `hello_mandatory` | `HELLO` first frame, carrying credentials |
| `helloStyle` | `map_payload` | `{version, token \| api_key, client_name}`; reply carries `proto` + `capabilities` |
| `push` | `reserved` | `PUSH_ID` is server→client only; emitting is opt-in |
| `maxFrameBytes` | `67108864` (64 MiB) | checked before allocation (WIRE-020) |
| `maxInFlight` | `256` | per-connection request bound |
| `errorCodes` | `both` | `"[code] message"` superset, recognizing `NOAUTH`/`WRONGPASS`/`NOPERM` |
| `tls` | `off` | additive capability, never a dialect |

An application that matches the standard writes its identity and nothing else. One that
still diverges says so **in its own repository**, where that knowledge belongs:

```ts
// A deployment whose RPC path authenticates via AUTH, has no HELLO handler,
// and ships a push-producing command.
const config = Config.standard()
  .withScheme("legacy")
  .withPort(15501)
  .withHandshake("auth_command")
  .withHelloStyle("not_used")
  .withPush("enabled");
```

Every `with*` returns a **new** frozen config, so convergence is visible and
per-application: delete overrides until only identity remains. Nothing waits on a Thunder
release. A config is data (PRO-003) — a plain object literal (or one loaded from your own
settings file, lifted with `Config.from`) works anywhere a `Config` is accepted; the
`with*` prefix exists only because TypeScript cannot give one object both a `scheme`
property and a `scheme()` method.

The standard's values are pinned to
[`conformance/standard.yaml`](../conformance/standard.yaml) by a test in every language,
so the four implementations can never disagree about what "standard" means.

## Errors

Every failure is a `ThunderError` with a stable `errorClass` — branch on the class (or
the subclass) and `code`, never on message text:

| Class | Subclass | Meaning |
|---|---|---|
| `auth` | `AuthError` | Handshake rejection, `NOAUTH`/`WRONGPASS`/`NOPERM` replies |
| `server` | `ServerError` | Server `Err` reply; `code` carries a parsed `"[code] "` prefix |
| `connection` | `ConnectionError` | Dial/write failure, connection died, client closed |
| `timeout` | `TimeoutError` | Connect or per-call timeout elapsed |
| `frame-too-large` | `FrameTooLargeError` | Inbound frame past the config's cap (connection poisoned) |
| `decode` | `DecodeError` | Malformed frame, or push under a `reserved` config (poisoned) |

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

## Browser

The package works in a browser bundle, but only the **wire layer** does — the
client opens sockets, reads files and negotiates TLS, which a browser cannot.

```ts
import { FrameReader, decodeResponseBody } from "@hivehub/thunder/wire";

const reader = new FrameReader();
reader.push(chunk);
for (let body = reader.nextBody(); body; body = reader.nextBody()) {
  if (body.length === 0) continue;   // keep-alive (WIRE-024)
  handle(decodeResponseBody(body));
}
```

Two ways to reach it, and you do not have to choose deliberately:

- **`@hivehub/thunder/wire`** — the explicit subpath. Says what you mean, works
  in every bundler and in Node.
- **`@hivehub/thunder`** — resolves to the same wire-only build under the
  `browser` export condition, so a bundler targeting the browser gets it
  automatically.

No aliasing of `fs`/`net`/`tls` is needed. If you added those aliases to work
around the previous build failure, you can remove them.

What you get: the frame codec, `FrameReader`, the `Value` model, the typed
errors and the config types. What you do not: `Client` and `Pool`. This mirrors
the Rust crate's `default-features = false`, which carves out the same pure
wire layer for the same reason — SPEC-001 WIRE-030 makes the wire layer pure in
every language, and the package now reflects that.

## Specs

Normative contracts: [SPEC-001 wire format](../docs/specs/SPEC-001-wire-format.md) ·
[SPEC-002 configuration](../docs/specs/SPEC-002-configuration.md) ·
[SPEC-003 client](../docs/specs/SPEC-003-client.md) ·
[SPEC-005 conformance](../docs/specs/SPEC-005-conformance.md) ·
[SPEC-006 packaging](../docs/specs/SPEC-006-packaging-release.md)

## License

Apache-2.0 — same as the rest of the HiveLLM family.
