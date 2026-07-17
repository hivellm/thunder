# Proposal: phase3_typescript-package

## Why
Three independently maintained TypeScript transports exist today, on three different msgpack libraries, and the TS lane is where the missing-frame-cap gap lives (analysis T-004): no existing TS SDK validates the length prefix before allocating. One `@hivellm/thunder` package collapses them and closes the cap gap by construction.

## What Changes
New npm package `@hivellm/thunder` under `typescript/`: wire + client only (every family server is Rust, T-009). Serialization is `@msgpack/msgpack` ^3 (WIRE-031 — not msgpackr; revisit behind the codec interface once the corpus proves equivalence). Value is the discriminated union all three products converged on (`{kind, value}` + factories), with Int = `bigint` (`number` accepted on input for safe ranges) and Bytes = `Uint8Array` emitted as msgpack bin. Frame reading via the streaming FrameReader pattern with the cap enforced before allocation. Full SPEC-003 client: demux Map by id, 3 handshake styles, 10 s connect / 30 s call timeouts, 2-attempt lazy reconnect, typed errors with prefix parsing, push hook, shared endpoint parser. Corpus loader runs in the default vitest run. ESM+CJS dual build via tsup, Node >= 18.

## Impact
- Governing spec: SPEC-001 (WIRE-001..040) - docs/specs/SPEC-001-wire-format.md; SPEC-003 (CLT-001..090) - docs/specs/SPEC-003-client.md
- PRD requirements: FR-01..FR-27
- DAG: T3.1; depends on G2; feeds T3.4–T3.6 (gate G3)
- Affected code: typescript/ (new package `@hivellm/thunder`)
- Breaking change: NO (new package; product SDKs swap onto it separately in T3.4–T3.6)
- User benefit: one TS codec/client instead of three, frame cap finally enforced in TS, uniform timeouts/reconnect/typed errors
