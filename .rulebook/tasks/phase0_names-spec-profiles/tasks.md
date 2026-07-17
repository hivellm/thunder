## 1. Implementation
- [ ] 1.1 Check availability + reserve crates.io `thunder-wire`/`thunder-client`/`thunder-server` (placeholder 0.0.1 publishes if needed); record fallbacks in SPEC-006
- [ ] 1.2 Decide npm org (`@hivellm` vs `@hivehub`), reserve `@hivellm/thunder`; reserve PyPI `hivellm-thunder`; reserve NuGet `HiveLLM.Thunder`
- [ ] 1.3 Transplant `rpc-wire-format.md` v1 into `docs/spec/` verbatim + provenance header; coordinate a pointer note in the Nexus repo
- [ ] 1.4 Author `conformance/profiles/*.yaml` per PRO-001/PRO-011 (synap, nexus, vectorizer, lexum): scheme, port, handshake, hello style, push, caps, in-flight, error convention, TLS
- [ ] 1.5 Verify whether any Vectorizer deployment runs RPC TLS; record the SRV-040 ordering decision (pull into M1 or keep P1)
- [ ] 1.6 Record every decision in `.rulebook/decisions/`

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation
- [ ] 2.2 Write tests covering the new behavior
- [ ] 2.3 Run tests and confirm they pass
