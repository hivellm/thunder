## 1. Implementation
- [ ] 1.1 Move the nexus-server RPC listener onto thunder-server with Profile::nexus(); dispatch-trait adapter reuses the existing dispatch/ modules unchanged (PKG-030 step 1) - command layer untouched
- [ ] 1.2 Relocate nexus-protocol/src/resp3/ into nexus-server as an internal module - server-internal parsing code is never published again (FR-61)
- [ ] 1.3 Rust SDK: replace the nexus-protocol path dep with thunder-wire + thunder-client from crates.io (PKG-020); add `pub type NexusValue = thunder_wire::Value;` so the public API is unchanged (PKG-021, NFR-04)
- [ ] 1.4 Wire the SDK transport onto thunder-client's demuxed connection - the SDK gains the pipelining its mutex single-flight transport lacks today (T-003); keep the NEXUS_SDK_TRANSPORT factory product-side
- [ ] 1.5 Full Nexus suite + Thunder corpus green against the swapped server and SDK (gate-G2 criterion)
- [ ] 1.6 `cargo publish --dry-run` on the SDK proves zero path dependencies and no product-protocol package (PKG-022)

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation
- [ ] 2.2 Write tests covering the new behavior
- [ ] 2.3 Run tests and confirm they pass
