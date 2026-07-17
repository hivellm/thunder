## 1. Implementation
- [ ] 1.1 TS SDK: add `@hivehub/thunder`, rewire the transport onto the Thunder client under the Nexus profile; delete `src/transports/codec.ts`, `rpc.ts`, `types.ts` (PKG-040); command map, endpoint factory, and public API unchanged (PKG-021, NFR-04)
- [ ] 1.2 Python SDK: same over `hivellm-thunder`; delete `nexus_sdk/transport/codec.py`, `rpc.py`, `types.py` (PKG-040)
- [ ] 1.3 C# SDK: same over `HiveLLM.Thunder`; delete `Transports/Codec.cs`, `RpcTransport.cs`, `Types.cs` - this removes the `Typeless` usage flagged in analysis T-004 (NFR-02)
- [ ] 1.4 Verify the missing-frame-cap gap is closed in all three SDKs: an oversized inbound frame is refused with a typed error before allocation (WIRE-020/021 via the Thunder clients)
- [ ] 1.5 All three SDK test suites green on the swapped internals (FR-62, NFR-04)
- [ ] 1.6 Minor version bump per SDK with release notes stating the behavioral upgrades explicitly - caps enforced, connect/call timeouts (PKG-041)

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation
- [ ] 2.2 Write tests covering the new behavior
- [ ] 2.3 Run tests and confirm they pass
