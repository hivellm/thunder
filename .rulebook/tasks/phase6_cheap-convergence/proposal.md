# Proposal: phase6_cheap-convergence

## Why
Three of the five behavioral dimensions — caps, push, error grammar — converge at near-zero interop
cost (BN-008/BN-010/BN-011): they are default values, an already-uniform wire reservation, or a
server-side string change no deployed client parses today. Bundling them keeps TLS and handshake (the
two the owner singled out) as focused standalone tasks while still delivering "most of the way to one
behavior" cheaply. This task also carries the finish-line mechanics (BN-020): as a behavioral profile
column reaches parity, promote it to a family constant so the profile ends up carrying only identity.

## What Changes
- **Caps**: make 64 MiB the family default `max_frame_bytes`, checked before allocation, configurable
  per deployment; a single configurable `max_in_flight` default. Confirm every language client
  already enforces the pre-allocation check (they do — T3). Expose the config knob everywhere the
  code hardcodes it. Nothing observable changes for existing traffic (Synap's SDKs already cap at
  64 — BN-006).
- **Push**: confirm every client ships the `PUSH_ID` hook and treats `PUSH_ID` as server→client-only
  (done in T3); demote the profile `push` flag from a behavioral dialect to a per-product capability
  bit ("this product ships a push-producing command"). A future family push feature then lands once.
- **Error grammar**: implement/verify the canonical `[CODE] message` superset — parsing on the
  client (accept both legacy bare-prefix and bracketed forms, incl. `NOPERM`) and the reference/spec
  for server emission. Since no deployed client parses either grammar, this is server-string-local
  and low-risk; legacy grammars stay decode-only tolerance.
- **Profile-field retirement (finish line)**: for each behavioral column now identical across all
  registered profiles (`max_frame_bytes`/`max_in_flight`/`tls`/`push`), promote it out of the
  per-product profile into a single family constant and drop the redundant field in a minor; update
  the conformance suite to assert ONE behavior for the converged columns, legacy forms tolerance-only.

Scope is Thunder-repo only (shared stack + conformance + profile registry). Flipping a specific
product server's default or emitting the canonical error strings inside a product is the owner's
manual per-product adoption.

## Impact
- Affected specs: SPEC-008 (caps/push/error sections), SPEC-001 (WIRE-020 caps), SPEC-002 (profile
  field retirement / PRO-002), SPEC-003 (CLT-050 error parsing)
- Affected code: rust/thunder-{wire,client,server} config defaults; the three client packages'
  error classifiers + cap config; conformance/profiles/*.yaml (field retirement) + the profile
  constants/tests in all four languages; conformance corpus (canonical error asserted, legacy
  tolerance)
- Breaking change: NO — cap default lowered only where the effective client ceiling is already 64;
  error-string change is unconsumed by clients; profile-field promotion with a family default is
  non-breaking (PRO-002)
- User benefit: caps/push/errors become one family behavior with no coordination; the profile shrinks
  toward identity-only; a future push feature lands once, not per-product
- Depends on: phase6_canonical-behavior-spec (SPEC-008 caps/push/error sections) and, for the
  error-token set, phase6_registry-errata (`NOPERM`). Field retirement for `tls`/handshake columns
  trails phase6_tls-optional / phase6_handshake-optional reaching parity.
