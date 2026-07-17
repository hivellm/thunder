## 1. Implementation
- [ ] 1.1 Package skeleton: PyPI `hivellm-thunder`, import `thunder_rpc` (PKG-010), sole runtime dep `msgpack` >=1.1 (WIRE-031); ruff + pytest wiring (PKG-002), ruff clean
- [ ] 1.2 Value as frozen dataclass `(kind, value)` over the 8 variants + factories/accessors per T-014 ergonomics
- [ ] 1.3 Wire codec: `packb(use_bin_type=True)` so Bytes emits bin (WIRE-010), array-encoded Request/Response (WIRE-012), externally-tagged forms incl. the `{"Ok":{"Str":…}}` nesting (WIRE-003), compact ints + f64 bit-pattern preservation (WIRE-014), PUSH_ID
- [ ] 1.4 Frame codec: cap validated against the length prefix before allocation (WIRE-020/021), partial input + multiple buffered frames (WIRE-022), typed FrameTooLarge/decode errors (WIRE-023); int-array Bytes normalized on decode (WIRE-011) - emit is bin-only
- [ ] 1.5 Sync client (threading background reader): full SPEC-003 contract - connect 10 s + TCP_NODELAY (CLT-001), 3 handshake styles (CLT-002/003), idempotent close (CLT-004), demux by id / serialized writes / backpressure / unknown-id drop / poison-on-malformed (CLT-010..014), per-call timeout 30 s (CLT-020), 2-attempt lazy reconnect without replay (CLT-030/031), typed errors with prefix parsing (CLT-050..052), push hook (CLT-060), endpoint parser (CLT-070/071)
- [ ] 1.6 Async client (asyncio): the same full contract with identical semantics (FR-28 - the two clients differ in idiom only, never in behavior); cancellation removes the pending entry (CLT-021)
- [ ] 1.7 Corpus loader (~50 LOC) walking conformance/vectors/ and asserting per mode, in the DEFAULT pytest run - never gated, never skipped (TST-020)

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation
- [ ] 2.2 Write tests covering the new behavior
- [ ] 2.3 Run tests and confirm they pass
