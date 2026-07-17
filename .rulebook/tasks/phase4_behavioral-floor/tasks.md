## 1. Implementation
- [ ] 1.1 Language-neutral scenario table under conformance/: scenario name, scripted server behavior, expected client outcome — the single source all four suites implement (CLT-090)
- [ ] 1.2 Scripted counterpart server harness: per-scenario behaviors (permuted replies, oversize frame, garbage frame, unknown-id reply, push injection, delayed reply, connection drop) reusable from every language's test runner
- [ ] 1.3 Pipelined out-of-order completion: N concurrent calls, replies permuted by the server, every pending call resolves with its own id's result
- [ ] 1.4 Oversize refusal: inbound frame with declared length over the cap refused WITHOUT allocating the body; typed frame-too-large error; connection poisoned
- [ ] 1.5 Connect timeout and call timeout scenarios fire as typed timeout-class errors at the configured deadlines
- [ ] 1.6 Reconnect: 2 attempts then typed connection failure; successful reconnect replays the profile handshake before pending traffic
- [ ] 1.7 Push routing per profile: push = Reserved receiving a PUSH_ID frame is a protocol error + poison; push = Enabled delivers the decoded Value to the registered hook (CLT-060)
- [ ] 1.8 Error-class mapping for both conventions: NOAUTH/WRONGPASS map to the auth class regardless of convention (CLT-051); "[code] " bracket-code parsed into code + class (CLT-050); assertions branch on class/code, never message text (CLT-052)
- [ ] 1.9 Unknown-id drop (response for an id never sent is discarded without disturbing pending calls) and poison-on-malformed-frame scenarios
- [ ] 1.10 Wire the suite into the default test run of all four languages; same scenarios, same assertions — green x4 is gate G4
- [ ] 1.11 Live interop smoke per TST-050: env-gated on THUNDER_LIVE_URL_SYNAP/NEXUS/VECTORIZER — connect, handshake per profile, PING-class call, one typed-error call, clean close, per language; skipped when unset, exercised on the release path

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation
- [ ] 2.2 Write tests covering the new behavior
- [ ] 2.3 Run tests and confirm they pass
