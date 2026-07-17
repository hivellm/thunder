# Proposal: phase6_registry-errata

## Why
The behavioral-normalization source sweep (docs/analysis/behavioral-normalization/, finding BN-023)
proved three cells of the profile registry describe the products incorrectly, verified against the
product RPC listeners on 2026-07-17. One of the three is not merely a doc error but a functional
gap: `synap.yaml` says `handshake: none`, so a Thunder client on the `synap` profile takes the
`None` handshake branch and never sends `AUTH` — meaning it **cannot authenticate against a Synap
started with `require_auth`**, even though Synap's RPC path fully supports `AUTH`
(`synap-server/src/protocol/synap_rpc/server.rs:170-249`, shared `UserManager`). Registry
correctness is owed regardless of whether the rest of normalization (phase6) ever runs; the profile
is the single source the per-language `Profile` constants are generated from, so a wrong cell
propagates to all four packages by construction.

## What Changes
Correct the registry and everything pinned to it, in ONE coordinated commit so the YAMLs and every
language's `Profile` constants + pinning tests never drift apart:
- `conformance/profiles/synap.yaml`: `handshake: none` → `auth_command`; `hello_style` stays null
  (Synap has an `AUTH` command but genuinely no `HELLO` handler). `none` described only the
  `require_auth = false` posture, which is deployment policy, not a handshake dialect.
- `conformance/profiles/nexus.yaml`: `hello_style: positional_version` → the arg-less/metadata form
  (RPC `HELLO` takes `[]` and returns a Map `{server,version,proto,id,authenticated}`,
  `nexus-server/.../rpc/dispatch/admin.rs:54-77`); positional `[Int(1)]` is the RESP3 HELLO.
- `conformance/profiles/vectorizer.yaml`: `tls: optional_rustls` → `reserved_config` (spec'd in
  VECTORIZER_RPC.md but never wired; `RpcConfig` has no TLS keys). No product runs RPC TLS today.
- `conformance/vectors/handshake-nexus-hello-request.yaml`: fix the request shape it pins (currently
  the RESP3 positional form, not the RPC arg-less form).
- Add `NOPERM` to the recognized auth-family error tokens (Synap emits it,
  `synap_rpc/server.rs:243-245`) in SPEC-002/SPEC-003 and every language's error classifier.
- Regenerate/adjust the `Profile` constants + profile-pinning tests in rust/thunder-wire,
  typescript, python, csharp to match the corrected YAMLs.
- Update SPEC-002 PRO-001 prose and the affected profile documentation.

Also worth fixing while here (found by the T3 agents): the registry YAMLs use bare `off`/`on`
scalars for `tls`, which YAML-1.1 loaders (PyYAML) parse as booleans — quote them (`"off"`) so every
language's loader agrees.

## Impact
- Affected specs: SPEC-002 (PRO-001), SPEC-003 (error tokens), the canonical wire spec's handshake
  table if it repeats the Nexus shape
- Affected code: conformance/profiles/{synap,nexus,vectorizer}.yaml, conformance/vectors/
  handshake-nexus-hello-request.yaml, rust/thunder-wire/src/profile.rs + tests/profiles.rs,
  typescript/src/profile.ts + tests, python/thunder_rpc/profile.py + tests, csharp/.../Profile.cs +
  tests, all four error classifiers (NOPERM)
- Breaking change: NO on the wire (bytes unchanged). It DOES change client behavior on the `synap`
  profile (the client will now send `AUTH` when credentialed) — a bug fix, not a regression: today
  that path cannot authenticate at all.
- User benefit: the profile registry finally tells the truth; a Thunder client can authenticate
  against a `require_auth` Synap; `NOPERM` is classified instead of falling through to a generic
  server error.
- Sequencing: apply AFTER the T3 language packages land (done — 23b912b/4bab514/b367f4b) so the
  YAMLs and all four sets of constants change together in one reviewable diff. Independent of the
  canonical-behavior spec (phase6_canonical-behavior-spec) — this is correctness of the CURRENT
  registry, not a normalization decision.
