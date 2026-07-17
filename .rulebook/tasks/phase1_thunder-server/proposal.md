# Proposal: phase1_thunder-server

## Why
All three family servers re-implement the same accept-loop shape (F-010). Synap's listener is the fastest in the family (analysis §7 T-027) while Nexus carries the operational features — and a double/triple-serialization anti-pattern in its metrics path. thunder-server combines the fastest hot path with Nexus's hardening behind one dispatch trait, so products integrate business logic only and inherit lifecycle, safety, handshake enforcement and metrics.

## What Changes
Port the hot path from SYNAP's listener: BufWriter on the write half with drain-then-flush burst coalescing (+23% committed in-family evidence), exactly ONE serialization per response — out-bytes measured from the written buffer, in-bytes from the decoder's frame_len, re-encoding for metrics banned (the Nexus anti-pattern) — set_nodelay, and a configurable idle timeout. Add Nexus's per-connection in-flight Semaphore with a configurable cap, lock-free atomic session auth with profile handshake enforcement (HelloMandatory first-frame rule / AuthCommand pre-auth allowlist / None), and the Dispatch trait (dispatch/authenticate/capabilities — credential validation stays product-side). Thunder constructs HELLO replies from ServerInfo + profile + hooks covering both family shapes (Nexus {server,version,proto,id,authenticated}, Vectorizer {protocol_version,capabilities}); PUSH_ID client frames are refused while a per-connection PushSender stays valid for the connection lifetime. Seven atomic metric series recorded after successful write, rpc.conn/rpc.req spans with slow WARN, error-formatting helpers for both family conventions, the SRV-050 integration suite, and an optional tokio-rustls feature.

## Impact
- Governing spec: SPEC-004 (SRV-001..050, hot path SRV-006..009) - docs/specs/SPEC-004-server.md
- PRD requirements: FR-40..FR-45
- DAG: T1.5 (gate G1); depends on phase1_thunder-wire (T1.1)
- Affected code: rust/thunder-server (new)
- Breaking change: NO (new crate; dispatch trait freezes at G1)
- User benefit: products integrate via one trait and get the family's fastest listener plus safety, auth enforcement and metrics for free — no more per-product server forks
