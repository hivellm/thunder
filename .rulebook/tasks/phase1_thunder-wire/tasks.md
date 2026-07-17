## 1. Implementation
- [x] 1.1 Crate skeleton: deps serde, rmp-serde 1.x, serde_bytes, thiserror; async read/write helpers behind a tokio feature - core stays I/O-free (WIRE-030)
- [x] 1.2 Value enum (8 variants) with Bytes bin-canonical via serde_bytes (WIRE-010); factories + accessors per T-014 ergonomics
- [x] 1.3 Request{id,command,args} / Response{id,result} array-encoded (WIRE-012); PUSH_ID; DEFAULT_MAX_FRAME_BYTES
- [x] 1.4 Codec: encode_frame, decode_frame_with_limit (cap before allocation, WIRE-020/021), partial-input handling + exactly-one-frame consumption (WIRE-022), typed FrameTooLarge/decode errors (WIRE-023)
- [x] 1.5 Read path returns the frame length alongside the value so consumers never re-encode for metrics (SRV-007 feed)
- [x] 1.6 Legacy decode tolerances: int-array Bytes normalized (WIRE-011); map-shaped Request accepted (WIRE-013) - emit paths never produce them
- [x] 1.7 Port the full Nexus test matrix: round-trip all variants incl. NaN bit pattern, i64 extremes, partial header/body, two-frames-in-buffer, oversize rejection pre-allocation, garbage bodies
- [x] 1.8 Regression test for the T-029 probe: bin and seq forms both decode; emit is bin-only

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [x] 2.1 Update or create documentation covering the implementation
- [x] 2.2 Write tests covering the new behavior
- [x] 2.3 Run tests and confirm they pass
