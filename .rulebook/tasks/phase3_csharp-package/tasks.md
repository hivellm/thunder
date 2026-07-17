## 1. Implementation
- [ ] 1.1 Package skeleton: NuGet `HiveLLM.Thunder`, net8.0, sole runtime dep `MessagePack` 2.5.x; `dotnet build -warnaserror` + test wiring (PKG-002, PKG-010)
- [ ] 1.2 Value model over the 8 variants + factories/accessors per T-014 ergonomics
- [ ] 1.3 Wire codec via low-level `MessagePackWriter`/`MessagePackReader` ONLY - `Typeless` forbidden (WIRE-031, NFR-02); the Vectorizer FrameCodec.cs approach producing canonical compact ints matching the golden vectors (WIRE-014); Bytes as bin (WIRE-010), array-encoded Request/Response (WIRE-012), externally-tagged forms incl. the `{"Ok":{"Str":…}}` nesting (WIRE-003), PUSH_ID
- [ ] 1.4 Frame codec: cap validated against the length prefix before allocation (WIRE-020/021), partial input + back-to-back frames (WIRE-022), typed FrameTooLarge/decode errors (WIRE-023); int-array Bytes normalized on decode (WIRE-011) - emit is bin-only
- [ ] 1.5 Multiplexing: `ConcurrentDictionary` + `TaskCompletionSource` demux with monotonic u32 ids skipping PUSH_ID (CLT-010), serialized writes (CLT-011), max_in_flight backpressure (CLT-012), unknown-id drop (CLT-013), poison-on-malformed-frame (CLT-014)
- [ ] 1.6 Lifecycle: connect timeout default 10 s + TCP_NODELAY (CLT-001), 3 handshake styles HelloMandatory/AuthCommand/None with typed auth failures (CLT-002/003), idempotent Dispose/Close failing in-flight calls (CLT-004)
- [ ] 1.7 Per-call timeout default 30 s, configurable per client and per call, plus per-request `CancellationToken` removing the pending entry on cancel (CLT-020/021, FR-22)
- [ ] 1.8 Lazy reconnect: 2 attempts with capped backoff, re-handshake per profile, no silent replay (CLT-030/031); typed errors with prefix parsing, NOAUTH/WRONGPASS → auth (CLT-050..052, WIRE-040); push hook (CLT-060); endpoint parser (CLT-070/071)
- [ ] 1.9 Corpus loader (~50 LOC) walking conformance/vectors/ and asserting per mode, in the DEFAULT `dotnet test` run - never gated, never skipped (TST-020)

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation
- [ ] 2.2 Write tests covering the new behavior
- [ ] 2.3 Run tests and confirm they pass
