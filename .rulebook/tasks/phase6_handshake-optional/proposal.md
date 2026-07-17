# Proposal: phase6_handshake-optional

## Why
The handshake is the one behavioral dimension whose divergence actually changes the first frame a
deployed client sends: three models coexist — Synap (bare `AUTH`, no HELLO), Nexus (optional
arg-less HELLO + separate `AUTH`), Vectorizer/Lexum (mandatory HELLO map). The
behavioral-normalization analysis shows they converge only if the canonical HELLO **shape** is
separated from auth **enforcement** (BN-012): "lead with HELLO" is about frame ordering and
negotiation, not about requiring credentials. Per the owner's directive, the canonical HELLO is
**implemented but optional** — the shared stack fully supports it (both sides), and each product
opts in via its profile; no product is forced to mandatory-HELLO, and auth enforcement stays a
per-deployment toggle. The source sweep also showed this is cheaper than feared: Synap's RPC path
already authenticates behind `require_auth`, so the work is "put a HELLO in front of existing auth,"
not "build RPC auth" (BN-017/BN-023).

## What Changes
Implement the canonical HELLO handshake as a first-class, optional capability in the shared stack
(SPEC-008 handshake section, SPEC-004 SRV-014, SPEC-003 CLT-002):
- **rust/thunder-server dual-accept**: on a fresh connection accept EITHER a leading canonical
  `HELLO` map (negotiate `proto`, reply with a capabilities Map) OR a legacy first frame (per the
  profile: Nexus optional-HELLO+`AUTH` allowlist; Synap immediate/`AUTH`), disambiguated by the
  first frame's command — no ambiguity. Auth enforcement stays the deployment toggle
  (`require_auth`/`auth_required`); a credential-less HELLO is valid on an open deployment. Add a
  legacy-first-frame counter for the eventual per-product deprecation (owner-run).
- **HELLO credential mapping**: the HELLO map's `token`/`api_key`/`[user,pass]` fields feed the same
  verification a product already runs (Nexus `AUTH` handler; Synap shared `UserManager`) — no new
  auth subsystem.
- **rust/thunder-client + the three client packages**: able to lead with the canonical HELLO map
  and consume the capabilities reply + negotiated `proto`; selected per profile (the existing
  CLT-002 handshake-style branch gains/confirms the canonical map form). Optional: a profile that
  keeps a legacy style still works.
- **Corpus + behavioral tests** for the canonical handshake and the dual-accept matrix.

Scope is Thunder-repo only: the shared capability + conformance. The per-product server dual-accept
rollout, the SDK default-flip, and the eventual legacy-path cut (a product major) are the owner's
manual per-product adoption, gated on telemetry — not part of this task.

## Impact
- Affected specs: SPEC-008 (handshake section), SPEC-004 (SRV-014 dual-accept + capabilities reply),
  SPEC-003 (CLT-002 canonical HELLO branch)
- Affected code: rust/thunder-server (listener handshake + dual-accept + capabilities reply hook),
  rust/thunder-client, typescript/src/client.ts, python/thunder_rpc/{client.py,aio.py,_handshake.py},
  csharp/.../ThunderClient.cs; conformance/vectors/ (canonical HELLO + tolerance)
- Breaking change: NO in Thunder — dual-accept keeps every legacy first-frame working; the canonical
  HELLO is opt-in per profile. (A product later making HELLO mandatory and cutting its legacy path
  is that product's own major, owner-run, out of scope here.)
- User benefit: one handshake implementation every product can adopt when it wants; auth enforcement
  stays where it belongs (deployment policy); a green-field product (Lexum) can be canonical from day
  zero; the fix also unblocks credentialed Synap access (see phase6_registry-errata)
- Depends on: phase6_canonical-behavior-spec (SPEC-008) and phase6_registry-errata (truthful Synap/
  Nexus handshake cells). Independent of phase6_tls-optional.
