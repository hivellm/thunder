## 1. Implementation
- [ ] 1.1 Lexum RPC listener on thunder-server with Profile::lexum() (HelloMandatory, MapPayload hello, error_codes Both, lexum:// port 17001 per PRO-011); dispatch-trait adapter over Lexum's command handlers - deps are thunder-wire + thunder-server from the registry (PKG-020, FR-11)
- [ ] 1.2 Rely on Thunder's profile enforcement: non-HELLO first frame rejected with the Both error convention (PRO-030, PRO-014) - no product-side handshake code
- [ ] 1.3 Wire the listener into Lexum's config/startup ([rpc] block, addr 127.0.0.1:17001 default, env overrides) mirroring its execution-plan shape minus the protocol crate
- [ ] 1.4 Write Lexum's SPEC-015 referencing Thunder's SPEC-001/SPEC-002 for bytes and profile semantics instead of respecifying them
- [ ] 1.5 Mark Lexum's planned P1 "create lexum-protocol" as superseded in docs/analysis/hivellm-rpc/05-execution-plan.md - a fourth wire-crate copy is never created (T-019)
- [ ] 1.6 Conformance under the lexum profile: handshake vector group + behavioral floor green (PRO-013); echo-level round-trip against thunder-client using lexum:// endpoints (gate-G2 criterion)

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation
- [ ] 2.2 Write tests covering the new behavior
- [ ] 2.3 Run tests and confirm they pass
