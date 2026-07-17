## 1. Implementation
- [ ] 1.1 Harness template with the three mandated scenarios — bulk ingest, small high-QPS call, pipelined polling — parameterized so any family product can point it at its own engine (BEN-040)
- [ ] 1.2 Same-handler discipline: RPC and HTTP paths invoke the identical product handler, so the engine cancels out and the transport is the only variable at product level (BEN-040)
- [ ] 1.3 Seed acceptance floors from Nexus's table (point read 320µs baseline -> ≤120µs target; bulk 780ms -> ≤220ms); mark them explicitly as seeds, never as results (BEN-040/BEN-031)
- [ ] 1.4 Run the harness on at least one product's real engine and commit the artifact (bench-out/-equivalent: JSON + md summary, env header) — the G4 requirement
- [ ] 1.5 Products calibrate their own floors from the first measured run; calibrated floors recorded alongside the artifact (BEN-040)
- [ ] 1.6 Roll the harness out to the remaining family products (Nexus, Vectorizer, Synap each on its own engine), one committed artifact per product
- [ ] 1.7 Claims discipline: no quantitative product claim without its committed artifact; numbers cite artifact path/commit; end-to-end never presented as transport-isolated (BEN-031)

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation
- [ ] 2.2 Write tests covering the new behavior
- [ ] 2.3 Run tests and confirm they pass
