## 1. Implementation
- [ ] 1.1 `conformance/profiles/synap.yaml`: `handshake: none` → `auth_command`, keep `hello_style: null`; update the inline comment to note `require_auth`-gated AUTH, no HELLO handler (BN-017/BN-023)
- [ ] 1.2 `conformance/profiles/nexus.yaml`: correct `hello_style` from `positional_version` to the arg-less/metadata-reply form; fix comment (positional `[Int(1)]` is RESP3, not RPC)
- [ ] 1.3 `conformance/profiles/vectorizer.yaml`: `tls: optional_rustls` → `reserved_config` (spec-only, unwired); quote all `tls` scalars (`"off"`) across the four YAMLs so YAML-1.1 loaders don't coerce to bool
- [ ] 1.4 `conformance/vectors/handshake-nexus-hello-request.yaml`: replace the RESP3 positional request with the actual RPC arg-less HELLO shape (or convert to decode-only tolerance if the positional form is worth keeping as legacy)
- [ ] 1.5 Add `NOPERM` to the recognized auth-family tokens in SPEC-002/SPEC-003 and confirm each language classifier maps it to the Auth class (rust error.rs already handles it; verify ts/py/cs)
- [ ] 1.6 Regenerate `Profile` constants + profile-pinning tests: rust/thunder (src/wire/profile.rs, tests/profiles.rs), typescript (profile.ts + test), python (profile.py + test — drop the PyYAML `off`→bool workaround once quoted), csharp (Profile.cs + test)
- [ ] 1.7 Update SPEC-002 PRO-001 prose (and the canonical wire-spec handshake table if it repeats the Nexus shape); confirm the Synap `synap` profile now drives an AUTH-capable handshake for credentialed deployments

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation — SPEC-002 PRO-001 + profile YAML comments reflect the corrected reality; cross-reference BN-023
- [ ] 2.2 Write tests covering the new behavior — profile-pinning tests in all four languages assert the corrected cells; a test proves the `synap` profile emits AUTH when credentials are configured
- [ ] 2.3 Run tests and confirm they pass — full gate green in all four languages (rust `cargo test -p thunder`, ts `npm test`, py `pytest`, cs `dotnet test`) against the corrected registry
