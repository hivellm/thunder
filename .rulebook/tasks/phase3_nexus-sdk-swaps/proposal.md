# Proposal: phase3_nexus-sdk-swaps

## Why
Nexus's TS, Python, and C# SDKs each carry a private codec + transport — three of the 18 copies the analysis counted (T-001), including two of the gaps it flagged (T-004): no inbound frame cap in the non-Rust transports and `Typeless` deserialization in C#. Swapping the internals onto the Thunder packages deletes that code and closes both gaps without touching the public API.

## What Changes
The three Nexus SDKs (`Nexus/sdks/{typescript,python,csharp}`) replace their transport internals with `@hivellm/thunder` / `hivellm-thunder` / `HiveLLM.Thunder` under the Nexus profile. Per-SDK codec/transport files are deleted, not kept as dead code (PKG-040): `src/transports/codec.ts` + `rpc.ts` + `types.ts`; `nexus_sdk/transport/codec.py` + `rpc.py` + `types.py`; `Transports/Codec.cs` + `RpcTransport.cs` + `Types.cs`. Command maps, endpoint factories, and public APIs stay unchanged (PKG-021, NFR-04). SDK suites must stay green; each SDK gets a minor version bump with release notes stating the behavioral upgrades explicitly (caps enforced, timeouts) per PKG-041.

## Impact
- Governing spec: SPEC-006 (PKG-040/041, PKG-021) - docs/specs/SPEC-006-packaging-release.md
- PRD requirements: FR-62, NFR-04
- DAG: T3.4; depends on T3.1 + T3.2 + T3.3; feeds gate G3
- Affected code: Nexus/sdks/typescript, Nexus/sdks/python, Nexus/sdks/csharp (external repo; net deletion of codec/transport sources)
- Breaking change: NO (internals-only; public API unchanged per PKG-021, minor bumps per PKG-041)
- User benefit: frame caps and timeouts finally enforced in all three Nexus SDKs, Typeless usage removed, three fewer codecs to maintain
