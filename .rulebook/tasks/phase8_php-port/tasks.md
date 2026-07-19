## 1. Implementation

- [x] 1.1 Prove the lane is possible before designing it: probed `rybakit/msgpack` against the corpus's pinned bytes — canonical PING frame, shortest-form ints, `bin` for Bytes, f64 bit patterns, bare `"Null"`, fixmap-of-1. All six matched exactly; recorded in the proposal
- [x] 1.2 `Value` — the 8 variants (WIRE-002) with an explicit kind, factory construction, typed accessors returning null on mismatch rather than throwing, and equality comparing floats **by bit pattern**. Tested both directions: NaN equals itself structurally, `-0.0` does not equal `0.0`
- [x] 1.3 `Request`/`Response` (WIRE-001) — array-encoded, `Response.result` as the nested `{"Ok"|"Err": …}` of WIRE-003
- [x] 1.4 Codec — externally-tagged encode/decode (WIRE-003/010/012/014) with both decode-only tolerances: `Bytes` as an int array (WIRE-011) and map-shaped `Request` with unknown keys skipped (WIRE-013). Each has a test asserting it decodes **and** is not re-emitted
- [x] 1.5 Framing — `u32` LE prefix, cap validated from the prefix **before** the "do I have the body?" question (proven with a header-only buffer), partial input as "need more bytes" without error, malformed body as a typed decode error, and WIRE-024's zero-length keep-alive: valid on the raw split, and on the typed path a `KeepAliveException` that extends `DecodeException` so it is distinct yet still catchable as one
- [x] 1.6 Typed errors — `FrameTooLargeException` and `DecodeException` as distinct classes, since the corpus's reject vectors assert the class rather than the message
- [x] 1.7 NOT PLANNED, found by static analysis: `unpackInt()` returns `GMP|Decimal|string` for integers above `PHP_INT_MAX`. The code assumed `int` and would have mangled a large `uint64`. Rejected as a decode error instead — and that is not a PHP workaround: Thunder's model is `Int(i64)` (WIRE-002), so such a value is outside the protocol in every lane

## 2. Tail (docs + tests — check or waive with tailWaiver)

- [x] 2.1 `php/README.md` following the Go lane's shape, and stating plainly that this is the wire layer only — the client is absent, not stubbed. Spec amendments landed: WIRE-031 gains PHP (and the Go row it was missing), SPEC-006 gains PKG-051
- [x] 2.2 Tests: the corpus loader (TST-020) walking `conformance/vectors/` and asserting all five modes, with the anti-shrink floor; plus unit tests for what the corpus cannot reach — the encode-side cap, partial-input handling, the keep-alive/malformed distinction, and the PHP-specific hazards (bytes vs text sharing one type, no unsigned integer)
- [x] 2.3 Gate green: PHPStan **level 9, zero errors** across `src` and `tests`; PHPUnit **60 tests, 314 assertions**, including all 39 corpus vectors. The loader runs in the default command — not gated, not skipped

## 3. Not in this task (each depends on this landing)

- The client: SPEC-003 CLT-001..090, including the mandatory pool (CLT-080)
- The interop probe and the CI lane in PKG-002 order
- Packagist publication, which needs the repository to exist first
