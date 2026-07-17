## 1. Implementation
- [ ] 1.1 TS SDK: rewire onto `@hivehub/thunder` under the Synap profile; delete the transport internals (PKG-040); command catalog + public API unchanged (PKG-021, NFR-04)
- [ ] 1.2 Python SDK: same over `hivellm-thunder`; requests switch from map-shaped to array-encoded (WIRE-012) - safe with no ordering constraint because the server tolerates both forms (WIRE-013)
- [ ] 1.3 C# SDK: same over `HiveLLM.Thunder`; delete Transport.cs including the hand-rolled MessagePack encoder (NFR-02, WIRE-031); requests become array-encoded (WIRE-012/013)
- [ ] 1.4 Reimplement the SUBSCRIBE/push flows over the CLT-060 push hook (dedicated-connection Synap-style subscription per CLT-061); user-facing subscription semantics unchanged
- [ ] 1.5 All three SDK test suites green on the swapped internals (FR-62, NFR-04)
- [ ] 1.6 Minor version bump per SDK with release notes stating the behavioral upgrades explicitly - caps enforced, timeouts, canonical request encoding (PKG-041)

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation
- [ ] 2.2 Write tests covering the new behavior
- [ ] 2.3 Run tests and confirm they pass
