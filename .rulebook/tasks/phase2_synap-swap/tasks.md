## 1. Implementation
- [ ] 1.1 Move synap-server onto thunder-server with Profile::synap() (handshake None, push Enabled); dispatch-trait adapter over the existing command handlers (PKG-030 step 1)
- [ ] 1.2 Register SUBSCRIBE flows against the per-connection PushSender - it remains valid after the registering request completes, so subscriptions emit for the connection lifetime (SRV-013, PRO-031)
- [ ] 1.3 Relocate synap-protocol's envelope.rs + resp3/ into synap-server as internal modules - never published again (FR-61)
- [ ] 1.4 Bytes canonicalization server-first (PKG-032): server emits bin (WIRE-010, FR-02) before or with the SDK swap; Thunder keeps decoding legacy int-array Bytes from old SDKs (WIRE-011)
- [ ] 1.5 Rust SDK: replace the synap-protocol path dep with thunder-wire + thunder-client from crates.io + one-line type alias (PKG-020/021, NFR-04) - the SDK gains true demux over its current hand-rolled transport
- [ ] 1.6 Verify old (pre-swap) Synap SDKs still pass their integration tests against the bin-emitting server; corpus tolerance vectors pin the legacy form (gate-G2 criterion)
- [ ] 1.7 Full Synap suite + Thunder corpus green; `cargo publish --dry-run` on the SDK proves zero path dependencies and no product-protocol package (PKG-022)

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation
- [ ] 2.2 Write tests covering the new behavior
- [ ] 2.3 Run tests and confirm they pass
