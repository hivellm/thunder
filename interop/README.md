# Live cross-language interop

Proof that a Thunder client in **every** language talks to a real Thunder
**server** over a socket — not just that the bytes match (the conformance corpus
already pins that), but that the connection, the standard `HELLO` handshake, and
`PING` / `ECHO` / typed-error round-trips all work end to end across languages.

The server is Rust-only by design (SPEC-004 — "Server (Rust)"), so the matrix is
**one server × four clients**, which is the real product topology: a Rust RPC
server with clients in Rust, TypeScript, Python and C#.

## Run it

```bash
python interop/run.py            # builds the Rust + C# probes, then runs
python interop/run.py --no-build # if the probes are already built
```

Expected tail:

```
  rust server  <-  rust        client : PASS  OK
  rust server  <-  python      client : PASS  OK
  rust server  <-  typescript  client : PASS  OK
  rust server  <-  csharp      client : PASS  OK

4/4 clients interoperate with the Rust server.
```

Exit code is 0 only if every client passes.

## The pieces

Each probe speaks `Config.standard()` (mandatory `HELLO` map + capabilities
reply) against an **open** server (no credentials), so the handshake itself is
part of what interops. Every probe uses its **local, uncommitted** Thunder
source/build, not the published package.

| Piece | File | Role |
|---|---|---|
| Rust server + client | [`rust/thunder/examples/interop.rs`](../rust/thunder/examples/interop.rs) | `cargo run --example interop -- <server\|client> <port>` |
| Python client | [`probe.py`](probe.py) | `python interop/probe.py client <port>` |
| TypeScript client | [`typescript/interop-probe.ts`](../typescript/interop-probe.ts) | `npx tsx interop-probe.ts client <port>` (from `typescript/`) |
| C# client | [`csharp/interop-probe/`](../csharp/interop-probe/) | `dotnet run --project csharp/interop-probe -- client <port>` |
| Driver | [`run.py`](run.py) | starts the Rust server per client, waits for `READY`, runs the client, reports |

A probe prints `OK` and exits 0 on success, or `FAIL: <why>` and exits 1.

## See it work at all (no matrix)

For the smallest end-to-end Thunder — one server, one client, in-process:

```bash
cargo run -p thunder-rpc --example hello
```
