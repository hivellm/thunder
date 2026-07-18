## 1. Implementation
- [ ] 1.1 **Design first, on stable Rust.** Evaluate: (a) generic `Dispatch<I>`; (b) `Principal` carrying `Option<Arc<dyn Any + Send + Sync>>` with a downcast accessor; (c) an associated type without a default (forces every impl to name it). Record the choice, what it breaks, and why — the issue's `type Identity = ()` sketch requires unstable `associated_type_defaults` and is not available
- [ ] 1.2 Implement the chosen shape so `authenticate` can return product data alongside the name
- [ ] 1.3 Carry it on the `Session` and expose it by reference — no clone per authorization check
- [ ] 1.4 Keep `Principal`'s existing `name` working for consumers that want nothing more
- [ ] 1.5 Document the semantics explicitly: identity is captured at AUTH and does not re-read live state, which is the pre-Thunder Synap behavior and the opposite of the workaround it was forced into

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation — SPEC-004 SRV-012 text, `rust/README.md` server section, and a migration note if 1.1 lands a breaking shape
- [ ] 2.2 Write tests covering the new behavior — a product payload survives the round trip to the session, is readable without cloning, and a `Dispatch` that wants no payload still compiles with minimal ceremony
- [ ] 2.3 Run tests and confirm they pass — full Rust gate green
