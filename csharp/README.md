# HiveLLM.Thunder

HiveLLM binary RPC for .NET — the frozen family wire v1 (length-prefixed
MessagePack) plus a multiplexed TCP client driven by a declarative protocol
config. Thunder is a protocol library: it ships one standard and zero product
knowledge, so any application — including one that does not exist yet —
configures itself without waiting on a Thunder release.

```
dotnet add package HiveLLM.Thunder
```

Targets `net8.0`. Sole runtime dependency: `MessagePack` 2.5.x (used through
the low-level `MessagePackWriter`/`Reader` API only).

## Quickstart

```csharp
using HiveLLM.Thunder;

// This caller's knobs: credentials, timeouts, client name.
var config = new ClientConfig
{
    Credentials = Credentials.ApiKey("secret-key"),
    ClientName = "my-service",
};
// The protocol config: the standard plus this application's own identity.
var app = Config.Standard() with { Scheme = "myapp", DefaultPort = 9000 };

await using var client = await ThunderClient.ConnectAsync(
    "myapp://localhost", app, config);

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

## Configuration

`Config` describes how **one application** uses the shared wire. It is data,
never behavior: no config may alter wire bytes — it selects among behaviors
the client already implements.

There is no registry and no per-product constants. `Config.Standard()` is
**the** family standard, every dimension is a knob, and identity is yours:

| Dimension | Standard | Meaning |
|---|---|---|
| `Scheme` | `""` | Your URL scheme — identity, Thunder has no default |
| `DefaultPort` | `0` | Your default RPC port — identity |
| `Handshake` | `HelloMandatory` | The only shape that negotiates `proto` and advertises capabilities |
| `HelloStyle` | `MapPayload` | `{version, token \| api_key, client_name}`; reply carries `proto` + capabilities |
| `Push` | `Reserved` | Emitting push is a capability you opt into |
| `MaxFrameBytes` | 64 MiB | Frame cap, checked before allocation |
| `MaxInFlight` | 256 | Per-connection request bound |
| `ErrorCodes` | `Both` | `[code] message` superset that also reads the RESP3 auth tokens |
| `Tls` | `Off` | Additive capability a deployment turns on, never a dialect |

The standard is pinned to `conformance/standard.yaml` by a test in every
language, so the four implementations can never disagree about what
"standard" means.

An application that matches the standard writes its identity and nothing
else; one that still diverges says so **in its own repository**:

```csharp
// A deployment whose RPC path authenticates via AUTH, has no HELLO handler,
// and ships a push-producing command.
var legacy = Config.Standard() with
{
    Scheme = "legacy",
    DefaultPort = 15501,
    Handshake = Handshake.AuthCommand,
    HelloStyle = HelloStyle.NotUsed,
    Push = PushPolicy.Enabled,
};
```

Convergence is visible and per-application: delete overrides until only
`Scheme` and `DefaultPort` remain. The record's `with` **is** the builder —
each override returns a new `Config` — and a plain `new Config { … }` works
just as well.

> `Config` is the **protocol** — the dialect, shared by everyone who talks to
> your application. `ClientConfig` is **this caller's** credentials, timeouts
> and client name, which never affect the dialect. `ConnectAsync` takes them
> in that order.

Endpoints accept `scheme://host[:port]`, where the scheme is your
`Config.Scheme` and a missing port resolves to your `Config.DefaultPort`, or
bare `host:port` (which needs no configured scheme). A scheme that is not
yours is rejected naming both. `http(s)://` is rejected — Thunder is
RPC-only; use your application's HTTP client for REST.

## Errors

Every failure is a `ThunderException` with a stable `ErrorClass` — branch on
the class (or catch the subclass), never on message text:

| Class | Exception | Meaning |
|---|---|---|
| `Auth` | `ThunderAuthException` | Handshake rejected; `NOAUTH`/`WRONGPASS`/`NOPERM` replies |
| `Server` | `ThunderServerException` | Server `Err` reply; `Code` carries a parsed `[code]` prefix |
| `Connection` | `ThunderConnectionException` | Dial/write failure, connection died, invalid endpoint |
| `Timeout` | `ThunderTimeoutException` | Connect or per-call timeout elapsed |
| `FrameTooLarge` | `ThunderFrameTooLargeException` | Inbound frame beyond the config's cap (checked before allocation) |
| `Decode` | `ThunderDecodeException` | Malformed frame; push frame under a push-reserved config |

## Push frames

Frames with `id == uint.MaxValue` are server push. Under
`PushPolicy.Enabled` register a hook with `client.OnPush(value => …)`; under
`PushPolicy.Reserved` — the standard — they poison the connection as a
protocol error.

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
  [SPEC-002 configuration](../docs/specs/SPEC-002-configuration.md) ·
  [SPEC-003 client](../docs/specs/SPEC-003-client.md) ·
  [SPEC-005 conformance](../docs/specs/SPEC-005-conformance.md)
- [`conformance/vectors/`](../conformance/) — the golden byte corpus every
  implementation asserts in its default test run

## License

Apache-2.0
