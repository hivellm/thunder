## 1. Implementation
- [ ] 1.1 Decide the payload type with the numbers in hand: `Arc<[u8]>` (issue option 1) vs `bytes::Bytes` (option 2). Record the decision and the reason — `bytes::Bytes` is already in most products' graphs and slices cheaply, `Arc<[u8]>` adds no dependency
- [ ] 1.2 Change `Value::Bytes` to the chosen type with a serde adapter emitting MessagePack `bin` exactly as today
- [ ] 1.3 Prove the wire is byte-identical: the full corpus passes unchanged, and the legacy int-array form still decodes (WIRE-011)
- [ ] 1.4 Update every construction and match site across wire, client, server and thunder-bench
- [ ] 1.5 Provide an ergonomic constructor path so `Value::from(vec)` / `Value::bytes(&[u8])` stays a one-liner for consumers that do own a `Vec`
- [ ] 1.6 Verify the zero-copy claim end-to-end: a decoded `Bytes` can move into a store as a refcount bump, and a stored buffer can reach the encoder without a copy

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation — SPEC-001 note that the Rust payload type is refcounted while the wire form is unchanged; migration note in `rust/README.md` and the root README
- [ ] 2.2 Write tests covering the new behavior — corpus unchanged, legacy tolerance intact, and a test that pins the no-copy property so a future refactor cannot silently reintroduce the memcpy
- [ ] 2.3 Run tests and confirm they pass — full Rust gate green, plus the cross-language interop run (the wire must be provably unchanged)
