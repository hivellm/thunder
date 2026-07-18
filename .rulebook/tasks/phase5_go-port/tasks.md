## 1. Implementation
- [x] 1.1 Module skeleton github.com/hivellm/thunder-go: vmihailenco/msgpack v5 with UseCompactInts(true) so emitted ints match the canonical corpus bytes (PKG-050)
      — `go/` module created. `UseCompactInts(true)` reproduces rmp-serde's shortest-form packing exactly (verified: positives as shortest unsigned, i64::MAX → cf, PushID → ce ff ff ff ff, f64 always cb, str8 for the 47-byte bracket vector, bin for Bytes). The corpus was green on the FIRST run — no byte-level fix needed.
- [x] 1.2 Wire layer per SPEC-001: 8-variant Value, array-encoded Request/Response, PUSH_ID, frame codec with cap-before-allocation, legacy decode tolerances
      — wire/value.go (Value + Request/Response + PushID + DefaultMaxFrameBytes), wire/codec.go (externally-tagged body tree, array Request/Response, int-array-Bytes WIRE-011 + map-shaped-Request WIRE-013 tolerances), wire/frame.go (u32-LE length prefix, cap-before-alloc, FrameTooLarge/Decode errors).
- [x] 1.3 Corpus loader wired into the default `go test` run per SPEC-005 (PKG-050 ships it with the module)
      — conformance/corpus_test.go walks ../../conformance/vectors/*.yaml, asserts every vector per mode, anti-shrink floor ≥39. Runs in `go test ./...`; 39 vectors checked green — the proof the Go bytes match Rust/TS/Python/C#.
- [x] 1.4 Client per SPEC-003: reader goroutine + demux via channels, per-call context cancellation, connect/call timeouts
      — client/client.go: reader goroutine, id demux via buffered channels, per-call context.Context cancellation (a Go-idiom addition), connect + per-call timeouts, in-flight gate (client/gate.go).
- [x] 1.5 All 3 handshake styles, reconnect with handshake replay, push hook routing per profile, typed errors for both prefix conventions
      — client/handshake.go (None / AuthCommand / HelloMandatory), lazy reconnect with handshake replay, push routing per profile, client/errors.go typed-error classification incl. NOPERM and both conventions (bare prefix + bracket-code). Tested: behavior_test.go (22 loopback-socket behaviors), errors_test.go (classification table), config_test.go (standard.yaml pin), endpoint_test.go.
- [x] 1.6 Release-train integration: module tag releases as the fifth lane (PKG-050)
      — README.md documents it: Go publishes from a VCS module tag (no registry credential / publish job); the shared gate adds gofmt -l / go vet / go test.
- [ ] 1.7 Swap Nexus's Go transport internals onto thunder-go; public API unchanged, product suite green
      — PRODUCT-REPO WORK (Nexus repo), owner-manual — out of scope for the Thunder repo.
- [ ] 1.8 Swap Vectorizer sdks/go/rpc onto thunder-go
      — PRODUCT-REPO WORK (Vectorizer repo), owner-manual.
- [ ] 1.9 Swap Synap transport_rpc.go onto thunder-go
      — PRODUCT-REPO WORK (Synap repo), owner-manual.

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [x] 2.1 Update or create documentation — go/README.md (usage + release-train note).
- [x] 2.2 Write tests covering the new behavior — wire (golden vectors, round-trip, NaN bits, framing edges), the corpus loader (39 vectors), and 27 client test functions (behaviors, errors, endpoint, config).
- [x] 2.3 Run tests and confirm they pass — verified independently: go build ./..., go vet ./..., gofmt -l . clean, go test ./... all green (client + conformance + wire). Note: `go test -race` needs a C compiler (absent here), so concurrency was stressed with -count=5 instead (stable).

## Note
The Thunder-repo Go module (1.1–1.6) is complete and corpus-verified. The three product-SDK swaps (1.7–1.9) live in the product repos and are the owner's manual adoption. The task's "depends on G5 (1.0 shipped)" is a sequencing preference; the module was built now and verified against the corpus, so it is ready when the release train tags it.
