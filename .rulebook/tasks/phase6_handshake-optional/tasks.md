## 1. Implementation
- [ ] 1.1 rust/thunder-server: dual-accept on fresh connection — branch on the first frame's command: canonical leading `HELLO` → negotiate `proto` + build capabilities reply; else legacy path per profile (Nexus optional-HELLO+`AUTH` allowlist; Synap `AUTH`/immediate). Disambiguated by command, no guessing (SRV-014)
- [ ] 1.2 rust/thunder-server: capabilities-reply hook (server supplies its command/capability list); `proto` negotiation integer in the reply; a credential-less HELLO is valid (enforcement = `require_auth`/`auth_required` toggle, unchanged)
- [ ] 1.3 rust/thunder-server: map HELLO credential fields (`token`/`api_key`/`[user,pass]`) onto the existing verification call; add a legacy-first-frame counter (telemetry for the owner's later cut)
- [ ] 1.4 rust/thunder-client: send the canonical HELLO map + consume capabilities/`proto`; selectable per profile; keep legacy handshake styles working (CLT-002)
- [ ] 1.5 typescript client: canonical HELLO map handshake branch + capabilities parse; per-profile selection; legacy styles intact
- [ ] 1.6 python client (sync + async, shared `_handshake.py`): same canonical branch + capabilities; identical semantics both clients
- [ ] 1.7 csharp client: same canonical branch + capabilities parse
- [ ] 1.8 Corpus: canonical HELLO request + capabilities reply vectors (from phase6_canonical-behavior-spec) asserted; add dual-accept behavioral coverage (server accepts canonical AND legacy first-frame)

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation — SPEC-004 SRV-014 + SPEC-008 handshake section reflect the dual-accept + capabilities reply; package READMEs show the canonical HELLO usage and note it is opt-in
- [ ] 2.2 Write tests covering the new behavior — per language: canonical HELLO round-trip (credentialed + credential-less), legacy first-frame still accepted (dual-accept), capabilities/`proto` surfaced, auth enforcement honors the deployment toggle
- [ ] 2.3 Run tests and confirm they pass — full gate green in all four languages (server dual-accept + client canonical handshake); conformance corpus passes
