## 1. Implementation
- [ ] 1.1 Caps: 64 MiB default `max_frame_bytes` (pre-allocation check) + single configurable `max_in_flight` default across rust/thunder-{wire,server,client}; expose the config knob wherever it is hardcoded; confirm the three client packages already do the pre-alloc check (T3) and take the configurable cap
- [ ] 1.2 Push: verify every client ships the `PUSH_ID` hook + server→client-only treatment (T3); demote the profile `push` field to a per-product capability bit (docs + profile comment), not a behavioral flag
- [ ] 1.3 Error superset: client-side parser accepts both legacy bare-prefix and bracketed `[CODE]` forms and classifies `NOAUTH`/`WRONGPASS`/`NOPERM`/`NOPROTO`; verify all four languages agree (rust reference already does — align ts/py/cs); legacy grammars remain decode-only
- [ ] 1.4 Reference/spec for canonical server emission (`[CODE] message`) so a product server has an exact target string when the owner migrates it; add canonical error corpus vectors (from phase6_canonical-behavior-spec) to the assertions
- [ ] 1.5 Profile-field retirement: for each behavioral column identical across all four profiles (`max_frame_bytes`, `max_in_flight`, `push`, and `tls`/handshake once their tasks reach parity), promote to a family constant and drop the per-product field (PRO-002); keep `scheme`/`default_port`/command-catalog identity fields
- [ ] 1.6 Update the conformance suite to assert ONE behavior for each converged column across all registered profiles; legacy forms retained as decode-only tolerance vectors

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation — SPEC-008 + SPEC-002 reflect the unified caps/push/error values and the retired profile fields; each package README notes the family defaults
- [ ] 2.2 Write tests covering the new behavior — cap default + override per language; `NOPERM` and both error-grammar forms classify correctly; profile-pinning tests assert the converged columns; a corpus assertion proves one canonical error behavior with legacy tolerated
- [ ] 2.3 Run tests and confirm they pass — full gate green in all four languages after the field retirement (profiles still load, constants match, corpus passes)
