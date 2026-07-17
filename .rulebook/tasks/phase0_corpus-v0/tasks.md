## 1. Implementation
- [ ] 1.1 Define the vector YAML schema exactly as TST-001 (name, group, mode, frame_hex, decoded, notes) and document it in conformance/README.md
- [ ] 1.2 Canonical group: PING request + nested PONG response vectors, hex copied from the Vectorizer spec/tests (TST-010)
- [ ] 1.3 Framing group: two-frames-in-buffer, partial header, partial body, zero-length body, frame at cap, frame at cap+1 with `mode: reject` (TST-012)
- [ ] 1.4 Schema validator script (any language, CI-runnable) that walks conformance/vectors/ and validates structure + hex parsing
- [ ] 1.5 Wire the validator into CI (corpus lane placeholder from T0.1 becomes real)

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation
- [ ] 2.2 Write tests covering the new behavior
- [ ] 2.3 Run tests and confirm they pass
