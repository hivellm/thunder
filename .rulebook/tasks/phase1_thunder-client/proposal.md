# Proposal: phase1_thunder-client

## Why
Vectorizer's Rust client is the only true multiplexer in the family (analysis T-028) — the others serialize calls and leave pipelining on the table. SPEC-003 defines a client contract that must hold identically in all four languages (NFR-07); the Rust client is the reference implementation, and its public API shape freezes at G1, so every later port copies this crate's semantics.

## What Changes
Build rust/thunder-client on the Vectorizer demux reference: a background reader task with oneshot demux by monotonically increasing u32 id (skipping PUSH_ID), serialized writes, and an in-flight bound from the profile with backpressure. All three profile handshake styles (None / AuthCommand optional HELLO+AUTH / HelloMandatory first-frame map with version+token/api_key+client_name), connect timeout 10 s + per-call 30 s, TCP_NODELAY, unknown-id drop, poison-on-malformed-frame, and 2-attempt lazy reconnect with capped backoff that never replays dead in-flight calls. Errors are typed with prefix parsing (NOAUTH/WRONGPASS to the auth class, "[code] " to a structured code); PUSH_ID frames go to a registered push hook; the endpoint parser comes from the profile registry and rejects http(s). Optional TLS behind a rustls feature; integration tests run against thunder-server echo under every profile.

## Impact
- Governing spec: SPEC-003 (CLT-001..080) - docs/specs/SPEC-003-client.md
- PRD requirements: FR-20..FR-27, FR-29; NFR-07
- DAG: T1.4 (gate G1); depends on phase1_thunder-wire (T1.1)
- Affected code: rust/thunder-client (new)
- Breaking change: NO (new crate; public API shape freezes at G1)
- User benefit: every family SDK inherits a true multiplexing client with uniform timeouts, reconnect, typed errors and push routing — the behavioral floor is built once
