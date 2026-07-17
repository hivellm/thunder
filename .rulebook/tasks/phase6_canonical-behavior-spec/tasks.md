## 1. Implementation
- [ ] 1.1 Draft `docs/specs/SPEC-008-canonical-behavior.md` skeleton (one section per dimension; requirement IDs e.g. `CAN-001..`), cross-linked to SPEC-001..005 and the behavioral-normalization analysis (BN-xxx)
- [ ] 1.2 Handshake section: pin the canonical HELLO-map shape (request fields + capabilities reply + `proto` negotiation) as an **optional, opt-in-per-profile capability**; write the **shape ≠ auth-policy** principle as a normative requirement; specify server dual-accept (canonical HELLO OR legacy first-frame, disambiguated by the first frame's command)
- [ ] 1.3 TLS section: one optional config/feature-gated `tokio-rustls` layer, off by default, no STARTTLS; define the config keys (cert_path/key_path) and the client opt-in; note it is the family's first running RPC TLS (BN-007/BN-009)
- [ ] 1.4 Error-grammar section: pin the `[CODE] message` superset spelling and the recognized token set incl. `NOPERM`; declare the two legacy grammars decode-only (BN-011)
- [ ] 1.5 Caps + push sections: 64 MiB configurable default (pre-allocation check), single configurable `max_in_flight`; `PUSH_ID` server→client-only, uniform client hook, emission = capability (BN-008/BN-010)
- [ ] 1.6 Add canonical corpus vectors: HELLO request + capabilities reply, `[CODE]` error forms (incl. `NOPERM`), following conformance/README.md schema; generate bytes from the reference encoder, never hand-computed
- [ ] 1.7 Add legacy-tolerance corpus vectors (decode/accept-only, `encode(decoded) != frame`): no-HELLO connect, arg-less HELLO, bare `NOAUTH`/`WRONGPASS`, 512 MiB cap config
- [ ] 1.8 Record the one open spelling decision (bracketed auth codes vs bare tokens alongside `[CODE]`) as DECIDED in the spec + a `.rulebook/decisions/` note

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation — SPEC-008 merged and cross-referenced from SPEC-002/003/004 and docs/specs/README.md
- [ ] 2.2 Write tests covering the new behavior — new corpus vectors load and assert per mode in the rust corpus test (canonical bidirectional, legacy decode-only)
- [ ] 2.3 Run tests and confirm they pass — `cargo test -p thunder-rpc` green with the new vectors present (canonical asserted, legacy tolerated)
