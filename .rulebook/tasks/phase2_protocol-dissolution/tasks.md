## 1. Implementation
- [ ] 1.1 nexus-protocol terminal shim: contents replaced by `#[deprecated]` re-exports of thunder-wire with the old type names (`pub type NexusValue = thunder_wire::Value;` etc.) + README deprecation notice pointing at thunder-wire (PKG-030 step 3)
- [ ] 1.2 vectorizer-protocol terminal shim: same recipe (PKG-030 step 3)
- [ ] 1.3 synap-protocol terminal shim: same recipe - envelope.rs/resp3/ already relocated by T2.3, only wire re-exports remain (PKG-030 step 3)
- [ ] 1.4 Publish the three shims to crates.io; each is the crate's last version ever - crates.io does not delete, shim-then-archive is the terminal state (PKG-031)
- [ ] 1.5 Delete crates/<product>-protocol from all three product workspaces; drop the protocol-publish step from each release pipeline permanently (PKG-030 step 4, FR-61)
- [ ] 1.6 Verify no in-repo consumer routes through a shim - servers and SDKs reference Thunder only, from the T2.1-T2.3 swap PRs (T-024)
- [ ] 1.7 Amended gate-G2 proof: every product Rust SDK passes `cargo publish --dry-run` with zero path dependencies and no product-protocol package (PKG-022, FR-62)

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation
- [ ] 2.2 Write tests covering the new behavior
- [ ] 2.3 Run tests and confirm they pass
