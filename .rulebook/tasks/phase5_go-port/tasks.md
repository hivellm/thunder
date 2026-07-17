## 1. Implementation
- [ ] 1.1 Module skeleton github.com/hivellm/thunder-go: vmihailenco/msgpack v5 with UseCompactInts(true) so emitted ints match the canonical corpus bytes (PKG-050)
- [ ] 1.2 Wire layer per SPEC-001: 8-variant Value, array-encoded Request/Response, PUSH_ID, frame codec with cap-before-allocation, legacy decode tolerances (int-array Bytes, map-shaped Request)
- [ ] 1.3 Corpus loader wired into the default `go test` run per SPEC-005 (PKG-050 ships it with the module)
- [ ] 1.4 Client per SPEC-003: reader goroutine + demux via channels, per-call context cancellation, connect/call timeouts
- [ ] 1.5 All 3 handshake styles (None / AuthCommand / HelloMandatory), reconnect with handshake replay, push hook routing per profile, typed errors for both prefix conventions (NOAUTH/WRONGPASS + bracket-code)
- [ ] 1.6 Release-train integration: module tag releases as the fifth lane (PKG-050)
- [ ] 1.7 Swap Nexus's Go transport internals onto thunder-go; public API unchanged, product suite green
- [ ] 1.8 Swap Vectorizer sdks/go/rpc onto thunder-go; public API unchanged, product suite green
- [ ] 1.9 Swap Synap transport_rpc.go onto thunder-go; public API unchanged, product suite green

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation
- [ ] 2.2 Write tests covering the new behavior
- [ ] 2.3 Run tests and confirm they pass
