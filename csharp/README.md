# HiveLLM.Thunder

HiveLLM binary RPC for .NET — the frozen family wire v1 (length-prefixed
MessagePack) plus a multiplexed TCP client driven by declarative product
profiles. One package serves Synap, Nexus, Vectorizer and Lexum.

```
dotnet add package HiveLLM.Thunder
```

Targets `net8.0`. Sole runtime dependency: `MessagePack` 2.5.x (used through
the low-level `MessagePackWriter`/`Reader` API only).

## Quickstart

```csharp
using HiveLLM.Thunder;

var config = new ClientConfig
{
    Credentials = Credentials.ApiKey("secret-key"),
    ClientName = "my-service",
};
await using var client = await ThunderClient.ConnectAsync(
    "vectorizer://localhost", Profile.Vectorizer, config);

var pong = await client.CallAsync("PING");
Console.WriteLine(pong.AsStr()); // "PONG"

var results = await client.CallAsync("SEARCH", new[]
{
    Value.Str("docs"),
    Value.Map((Value.Str("limit"), Value.Int(10))),
});
```

Calls multiplex concurrently over one connection; per-call timeout defaults
to 30 s (override per client or per call), `CancellationToken` is honored,
and a dead connection lazily reconnects (2 attempts, capped backoff) without
ever replaying in-flight calls.

## Profiles

Profiles are data, not behavior — they select among behaviors the client
already implements and are pinned to `conformance/profiles/*.yaml` by tests.
Custom profiles are plain `new Profile { … }` constructions.

| Profile | Endpoint scheme | Default port | Handshake | Push | Error convention |
|---|---|---|---|---|---|
| `Profile.Synap` | `synap://` | 15501 | `AUTH` (no `HELLO`) | enabled | RESP3 prefixes |
| `Profile.Nexus` | `nexus://` | 15475 | optional arg-less `HELLO` + `AUTH` | reserved | RESP3 prefixes |
| `Profile.Vectorizer` | `vectorizer://` | 15503 | mandatory `HELLO` map | reserved | `[code] message` |
| `Profile.Lexum` | `lexum://` | 17001 | mandatory `HELLO` map | reserved | both |

Endpoints accept `scheme://host[:port]` (port defaults from the registry) or
bare `host:port`. `http(s)://` is rejected — Thunder is RPC-only; use the
product's HTTP client for REST.

## Errors

Every failure is a `ThunderException` with a stable `ErrorClass` — branch on
the class (or catch the subclass), never on message text:

| Class | Exception | Meaning |
|---|---|---|
| `Auth` | `ThunderAuthException` | Handshake rejected; `NOAUTH`/`WRONGPASS`/`NOPERM` replies |
| `Server` | `ThunderServerException` | Server `Err` reply; `Code` carries a parsed `[code]` prefix |
| `Connection` | `ThunderConnectionException` | Dial/write failure, connection died, invalid endpoint |
| `Timeout` | `ThunderTimeoutException` | Connect or per-call timeout elapsed |
| `FrameTooLarge` | `ThunderFrameTooLargeException` | Inbound frame beyond the profile cap (checked before allocation) |
| `Decode` | `ThunderDecodeException` | Malformed frame; push frame under a push-reserved profile |

## Push frames

Frames with `id == uint.MaxValue` are server push. Under push-enabled
profiles (Synap) register a hook with `client.OnPush(value => …)`; under
reserved profiles they poison the connection as a protocol error.

## Wire layer

`FrameCodec` / `Value` / `Request` / `Response` are the pure byte-level API
(no sockets): `FrameCodec.EncodeRequest`, `FrameCodec.TryDecodeResponse`
(returns `false` on partial input, throws typed errors on cap violation or
malformed bodies), with the frame cap enforced against the length prefix
before any body allocation.

## Specs

The contract lives in the repo's language-neutral assets — this package is
one of four lockstep implementations:

- [SPEC-001 wire format](../docs/specs/SPEC-001-wire-format.md) ·
  [SPEC-002 profiles](../docs/specs/SPEC-002-profiles.md) ·
  [SPEC-003 client](../docs/specs/SPEC-003-client.md) ·
  [SPEC-005 conformance](../docs/specs/SPEC-005-conformance.md)
- [`conformance/vectors/`](../conformance/) — the golden byte corpus every
  implementation asserts in its default test run

## License

Apache-2.0
