# Proposal: phase6_client-pool

## Why

**CLT-080 is a promise the implementation never kept.** SPEC-003 §8 says an optional pool "MAY ship
per language, mirroring the Vectorizer pattern". No language ships one. The spec even names the
consequence: "until CLT-080 lands, the swap keeps a thin product-side pool wrapper over Thunder
clients (the existing ~150-LOC pattern)" — i.e. every adopting product re-implements the same pool,
in its own SDK, in four languages. That is exactly the duplication Thunder exists to end.

**The family has already paid for not pooling — twice, with the same root cause.**

- `Nexus/crates/nexus-protocol/src/rest.rs:14-33` — a fresh client per request gave every call its
  own empty pool, forcing a new TCP connection every time. It exhausted Windows ephemeral ports via
  TIME_WAIT and forced a "batch-40 + retry + item-by-item" workaround, deleted once the client was
  built once and reused (`b0615d34`).
- `Vectorizer` commit `395f7b25` (`fix(raft): use lazy channel pooling instead of connect-per-RPC`) —
  a full HTTP/2 handshake per vote/append/snapshot: *"'transport error' when peer gRPC wasn't ready
  during handshake; Hundreds of TIME_WAIT connections accumulating; No connection reuse between
  RPCs."* Shipped as a release fix (`64660468`, v2.5.7).

The mechanism in both cases is the one pooling addresses: **connect + handshake paid per operation**.
Thunder makes that *worse* than plain TCP, because `Handshake::HelloMandatory` + `auth_required` mean
a new connection costs a round trip before the first request. `Vectorizer/sdks/rust/src/rpc/pool.rs`
already exists in four languages for precisely this reason — its proposal says "Connection pooling
(`RpcPool`) to amortize TCP + auth handshakes."

**On the one number in the family, an honest note:** `Vectorizer/docs/patches/v0.10.0-0.10.9.md:260`
claims "Connection Pooling: Reduced connection overhead by 80%". No benchmark, artifact or raw data
backs it — it is changelog prose, in a list beside "300% GRPC vs HTTP" and "500% binary
serialization". It is cited here as the origin of the memory, **not** as evidence. This task is
justified by the two root-caused failures above, not by that figure.

## What Changes

Port the Vectorizer `RpcPool` shape into `thunder`'s client, in all four languages, as an **optional
layer above** the single-connection client (CLT-001: "pooling is a layer above, CLT-080"). The
single-connection client stays the primary contract and its API does not change.

Shape, per `Vectorizer/sdks/rust/src/rpc/pool.rs` (149 LOC) and its siblings (`pool.ts` 132,
`pool.py` 254, `pool.go` 171):

- fixed `max_connections`, bounded by a semaphore permit;
- an idle list; `acquire()` hands back an RAII guard that returns the connection on drop/close;
- lazy connect — a slot connects on first checkout, not at construction;
- **explicitly not** bb8/deadpool/r2d2. The Vectorizer header states the rejection: *"those bring
  async traits and heavyweight reconnect logic that the v1 SDK doesn't need."* Same call here.

Per-language idiom for the guard is the deliberate variation: Rust `Drop`, C# `IDisposable`/`await
using`, Python context manager, TypeScript explicit `release()` in a `finally`.

**Two spec items must be settled before code:**

1. **CLT-080 is [P1] and says "MAY".** Shipping it in one language and not another is worse than not
   shipping it — that is the per-product fork this repo exists to prevent. Either it lands in all
   four or it stays unbuilt. This task proposes promoting it to **[P0] "SHALL, in every language"**.
2. **CLT-080's neighbours are stale.** It sits beside CLT-070/071, which still route through "the
   profile registry" — deleted by `phase6_agnostic-config`. The pool takes a `Config` like every
   other entry point; no registry, no scheme table, no product names (PRO-010/012).

**Explicitly out of scope**: pool health checks, background reaping, min-idle warmup, and any
lifecycle beyond checkout/return. If a connection is poisoned (CLT-014), the slot is dropped and the
next checkout connects fresh — reconnect stays CLT-030's job, not the pool's.

## Impact

- Affected specs: SPEC-003 (CLT-080; CLT-001 layering; CLT-012 in-flight bound; CLT-014 poison;
  CLT-030 lazy reconnect), SPEC-002 (PRO-010 no product names)
- DAG: independent of `phase4_hotpath-optimization` — **this does not touch a G5 cell**. The matrix
  reuses connections, so no cell exercises checkout; `connection-storm` is a stable tie across all
  three runs (−0.3% / −1.9% / +0.1%) because its cost is the kernel's, not the wire's. Claiming a
  pooling win from the current matrix would violate BEN-031.
- Affected code: `rust/thunder/src/client/` (new `pool.rs`, behind the existing `client` feature),
  `typescript/src/rpc/`, `python/thunder/`, `csharp/Thunder/`; `docs/specs/SPEC-003-client.md` §8
- Breaking change: NO — additive. The wire is untouched; `Client` keeps its API; the pool is opt-in.
- User benefit: the ~150-LOC pool Vectorizer maintains in four languages, and that Nexus and
  Vectorizer's Raft each discovered the hard way through TIME_WAIT exhaustion, gets written once and
  tested once. A product opening a connection per operation pays a handshake round trip per operation
  under Thunder's mandatory HELLO — this is the layer that stops that.
