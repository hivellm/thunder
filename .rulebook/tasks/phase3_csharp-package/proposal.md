# Proposal: phase3_csharp-package

## Why
C# is where the family's riskiest serialization strategies live: three different approaches today, including `MessagePackSerializer.Typeless` (a deserialization risk, analysis T-004) and a hand-rolled codec. One `HiveLLM.Thunder` package on the low-level MessagePack API retires both (NFR-02) and gives C# the full client floor including first-class cancellation (FR-22).

## What Changes
New NuGet package `HiveLLM.Thunder` under `csharp/`, targeting net8.0: wire + client only. Serialization is `MessagePack` 2.5.x using ONLY the low-level `MessagePackWriter`/`MessagePackReader` — `Typeless` is forbidden (WIRE-031, NFR-02); this is the Vectorizer approach (FrameCodec.cs), which produces canonical compact ints matching the golden vectors. Full SPEC-003 client: `ConcurrentDictionary` + `TaskCompletionSource` demux, connect (10 s) + per-call (30 s) timeouts plus per-request `CancellationToken` (CLT-021), 3 handshake styles, 2-attempt reconnect, typed errors, push hook, endpoint parser. Corpus loader runs in the default `dotnet test` run.

## Impact
- Governing spec: SPEC-001 (WIRE-001..040) - docs/specs/SPEC-001-wire-format.md; SPEC-003 (CLT-001..090) - docs/specs/SPEC-003-client.md
- PRD requirements: FR-22, NFR-02 (over the FR-01..FR-27 floor)
- DAG: T3.3; depends on G2; feeds T3.4–T3.6 (gate G3)
- Affected code: csharp/ (new package `HiveLLM.Thunder`)
- Breaking change: NO (new package; product SDKs swap onto it separately in T3.4–T3.6)
- User benefit: canonical bytes with zero Typeless deserialization risk, per-request cancellation, uniform caps/timeouts/reconnect
