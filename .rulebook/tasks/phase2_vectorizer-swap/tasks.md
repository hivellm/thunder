## 1. Implementation
- [ ] 1.1 Move the Vectorizer RPC listener onto thunder-server with Profile::vectorizer(); dispatch-trait adapter over the existing command handlers (PKG-030 step 1)
- [ ] 1.2 Supply the HELLO capabilities reply through the dispatch capabilities() hook - Thunder builds the reply, product code only provides the list (SRV-014)
- [ ] 1.3 Honor the T0-recorded TLS decision via the profile's tls dimension - no product-side TLS plumbing left in the listener
- [ ] 1.4 Rust SDK: replace the vectorizer-protocol path dep with thunder-wire + thunder-client from crates.io (PKG-020) + one-line type alias keeping the public API (PKG-021)
- [ ] 1.5 Keep the thin product-side pool wrapper over Thunder clients until CLT-080 ships (SPEC-003 swap note) - the swap is never blocked on the pool requirement
- [ ] 1.6 Retain Vectorizer's golden vector tests as the transition double-check; full suite + Thunder corpus green (gate-G2 criterion)
- [ ] 1.7 `cargo publish --dry-run` on the SDK proves zero path dependencies and no product-protocol package (PKG-022)

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation
- [ ] 2.2 Write tests covering the new behavior
- [ ] 2.3 Run tests and confirm they pass
