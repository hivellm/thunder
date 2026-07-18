# Proposal: phase7_bytes-zero-copy (issue #4 of the Synap adoption set, GH #1)

## Why
`thunder::Value::Bytes` owns a `Vec<u8>`, so adopting Thunder cost Synap the
zero-copy path it had built deliberately (its phase11 wire-value and phase13
parse-bulk-into-arc work), on the very path behind the family's
`SynapRPC ≈ 3× RESP3` benchmark.

Synap's pre-Thunder type held `Bytes(Arc<[u8]>)` and serialized identically, so
the wire never changed while the in-process path stayed copy-free both ways:
`SET`'s decoded argument moved into the store as a refcount bump, and
`GET`/`MGET` carried the store's `Arc<[u8]>` straight to the encoder.

Now both directions `memcpy` the full value. The cost scales with payload size,
so it is worst exactly where a binary protocol is supposed to win: large values
and the raw-LE-f32 embeddings the README advertises.

## What Changes
`Value::Bytes` carries a refcounted buffer instead of an owned `Vec<u8>` —
`Arc<[u8]>` with a `serde_bytes`-equivalent adapter (the issue's option 1, and
the closest to the current design).

**The wire does not change.** The emitted form stays MessagePack `bin`
(WIRE-010) and the legacy int-array form stays tolerated on decode (WIRE-011),
so no corpus vector changes and no other language lane is affected.

## Impact
- Governing spec: SPEC-001 (WIRE-010 / WIRE-011 — both preserved)
- Affected code: rust/thunder/src/wire/ (Value, codec), every construction and
  match site across client, server and bench
- Breaking change: **YES — public API.** The wire is untouched; the Rust type
  is not. `Value::Bytes(vec)` and `match … Value::Bytes(b) => b.as_slice()`
  change shape for every consumer.
- Ships in: **0.2.0, not 0.1.2** — see the note below
- User benefit: Synap recovers its zero-copy GET/SET; every product with large
  payloads stops paying a full copy per call in each direction

## Why 0.2.0 rather than 0.1.2
0.1.2 is a patch release; this changes a public enum's payload type, which
every consumer pattern-matches on. Under the project's own semver rule
(PKG-012: "public API breaks = major", which at 0.x means the minor) it belongs
in 0.2.0. Shipping it as a patch would break builds for anyone who upgraded
expecting a fix release.
