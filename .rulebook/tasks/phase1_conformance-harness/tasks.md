## 1. Implementation
- [ ] 1.1 Value group: every variant alone and nested; empty Bytes/Str/Array/Map; Map with non-string keys; i64::MIN/MAX and compact-int boundaries (-32, 127, 255, 65535, ...); f64 NaN bit pattern, ±∞, -0.0; Err plain, Err with "[code] " prefix, NOAUTH/WRONGPASS strings (TST-011)
- [ ] 1.2 Framing group completion: two frames in one buffer, partial header, partial body, zero-length body, frame at exactly the cap, frame one byte over the cap as reject with no-allocation assertion (TST-012)
- [ ] 1.3 Tolerance group (decode-only): Bytes as int-array (Synap legacy) and map-shaped Request (TST-013); push group vector with id = u32::MAX (TST-014)
- [ ] 1.4 Handshake group: Nexus HELLO [Int(1)] positional request/reply shape; Vectorizer HELLO map (version/token/api_key/client_name) and capabilities reply (TST-015)
- [ ] 1.5 Rust corpus loader (~50 LOC) walking conformance/vectors/ and asserting per mode (bidirectional/decode-only/reject), wired into the DEFAULT test run - no feature gates, no #[ignore] (TST-020, NFR-03)
- [ ] 1.6 Reference cross-decode: nexus-protocol as dev-dependency, both directions over the canonical + value groups - Thunder frames decode via nexus_protocol::rpc into equal structures and vice versa (TST-030)
- [ ] 1.7 Pairwise-fuzz seed generator: deterministic per seed, random Value trees as JSON for encode/decode/re-encode agreement; auto-shrink divergences to the shortest failing tree and graduate fixes into new corpus vectors (TST-040, TST-041)
- [ ] 1.8 CI lanes: corpus per PR + nightly rolling-seed fuzz (TST-021, TST-040)
- [ ] 1.9 Coverage check: every SPEC-001 MUST maps to ≥1 vector, recorded in the vector's notes field (TST-016)

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation
- [ ] 2.2 Write tests covering the new behavior
- [ ] 2.3 Run tests and confirm they pass
