## 1. Implementation
- [x] 1.1 Caps: 64 MiB default `max_frame_bytes` (pre-alloc check) + single configurable `max_in_flight` default; expose the knob; all four clients do the pre-alloc check
      — ALREADY CONVERGED: `Config::standard()` pins `max_frame_bytes` 64 MiB and `max_in_flight` 256 (SPEC-002 PRO-011), both configurable per app. The pre-allocation cap check is normative (WIRE-020) and corpus-pinned by `framing-cap-plus-one` / `framing-over-cap-with-body`, which ALL FOUR language corpus loaders run and pass. Nothing hardcodes a different value.
- [x] 1.2 Push: every client ships the PUSH_ID hook + server→client-only; demote the push flag to a capability bit
      — ALREADY CONVERGED: PUSH_ID is server→client-only with a uniform client hook (CLT-060), exercised by the four-language behavioral floor (push=Reserved poisons, push=Enabled delivers). `push` is a config knob (PushPolicy Reserved/Enabled), framed as a per-product capability in SPEC-008 CAN-050 — not a dialect.
- [x] 1.3 Error superset: client parses bare-prefix + bracketed [CODE], classifies NOAUTH/WRONGPASS/NOPERM/NOPROTO; all four agree; legacy decode-only
      — ALREADY CONVERGED: verified all four error classifiers recognize exactly `{NOAUTH, WRONGPASS, NOPERM}` as auth-class (rust error.rs, ts errors.ts, py errors.py, cs Errors.cs) and parse both bare-prefix and bracketed `[CODE]`. NOPERM is corpus-pinned by `response-err-noperm` (runs x4). NOPROTO: found it is in NO implementation's set, and clarified in SPEC-008 CAN-031 that it is NOT auth-class (protocol-negotiation failure, no path emits it) — the spec now matches the four implementations rather than overclaiming. The WRONGPASS-over-the-wire tests (both conventions) were added to all four this session.
- [x] 1.4 Reference/spec for canonical server emission + canonical error corpus vectors
      — DONE: SPEC-008 CAN-030 pins the emission spelling (bare auth tokens + bracketed `[CODE]`, decision recorded); `response-err-noperm` added to the corpus (bytes from the reference encoder), round-trips x4.
- [~] 1.5 Profile-field retirement: promote each converged column to a family constant, drop the per-product field
      — MOOT / ALREADY DONE by the agnostic-config refactor (ec6be5a), which deleted the entire per-product profile registry (`conformance/profiles/*.yaml`, `Profile::synap()` etc.) and replaced it with ONE `Config::standard()` + `conformance/standard.yaml`. There are no per-product profile columns left to retire — the family constant IS the standard. This task item presupposed the registry this repo has already dissolved.
- [x] 1.6 Conformance asserts ONE behavior per converged column; legacy forms decode-only tolerance
      — DONE: the corpus pins the canonical shapes as `bidirectional` and the genuinely-legacy forms (int-array Bytes, map-shaped Request) as `decode-only` (WIRE-011/016); all four loaders assert identically (CAN-090).

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [x] 2.1 Docs — SPEC-008 (caps CAN-040, push CAN-050, error CAN-030/031/032) and SPEC-002 PRO-011 carry the unified values; the profile-field retirement is already reflected (no registry). CAN-031 tightened this session to match the four implementations.
- [x] 2.2 Tests — cap default + override (behavioral floor + corpus, x4); NOPERM + both grammar forms classify correctly (x4, incl. the new WRONGPASS-over-the-wire tests); the corpus proves one canonical error behavior with legacy tolerated.
- [x] 2.3 Run tests — the four-language suites are green (Rust/TS/Python/C# floor + corpus); no field retirement needed (already done), so nothing to re-load.

## Note
Like phase6_handshake-optional, this task was largely pre-satisfied: the agnostic-config refactor already retired the per-product profiles (1.5), SPEC-008 pinned the caps/push/error values, and the four-language behavioral floor + corpus already assert the converged behavior. The genuinely-new work this session was the SPEC-008 CAN-031 NOPROTO precision fix and the `response-err-noperm` corpus vector; both landed. Emitting the canonical strings inside a product server is the owner's per-product adoption, out of scope.
