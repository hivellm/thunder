# Proposal: phase6_tls-optional

## Why
No product runs RPC TLS today: Vectorizer's `optional_rustls` is spec'd but never wired (its
`RpcConfig` has no TLS keys), Nexus documents an LB/sidecar posture, Synap and Lexum have none
(BN-007/BN-023). TLS is therefore the one behavioral dimension that is a *missing capability*, not a
*conflicting behavior* — shipping it once, uniformly, is purely additive and breaks no deployed
client. Per the owner's directive, TLS is **implemented but optional**: available to every profile,
off by default, turned on per deployment when a project needs it. This makes Thunder's the family's
first running RPC TLS.

## What Changes
Implement one optional, config/feature-gated `tokio-rustls` transport layer across the shared stack,
off by default, no STARTTLS (TLS decided at connect time), matching SPEC-004 SRV-040 (server) and
FR-29 (client) and the SPEC-008 TLS section:
- **rust/thunder-server**: wrap the listener's accepted stream in a `TlsAcceptor` when
  `tls.cert_path`/`tls.key_path` are configured; feature-gate the rustls dependency; plaintext path
  unchanged when unset.
- **rust/thunder-client**: optional TLS connector (rustls/native roots or a configured CA), opt-in
  via endpoint/profile config; plaintext default preserved.
- **typescript / python / csharp client packages**: a TLS connect option (Node TLS socket / Python
  `ssl` context / .NET `SslStream`) gated by client config, off by default, mirroring the Rust
  client's knobs and error surface (a TLS failure classifies as Connection).
- Docs + a profile-level note that `tls` is deployment config, not product identity.

Scope is Thunder-repo only: the shared capability. Turning it on inside a specific product server or
routing a product SDK through it is the owner's manual per-product adoption, not part of this task.

## Impact
- Affected specs: SPEC-004 (SRV-040), SPEC-003/PRD (FR-29), SPEC-008 (TLS section)
- Affected code: rust/thunder-server (listener), rust/thunder-client (connector),
  typescript/src/client.ts, python/thunder_rpc/{client.py,aio.py}, csharp/.../ThunderClient.cs;
  config structs in each; feature flags / optional deps (rustls, and the stdlib TLS in the three
  client languages)
- Breaking change: NO — off by default; an off TLS option cannot break a plaintext client. Adding an
  optional dependency/feature is non-breaking.
- User benefit: any project can encrypt its RPC transport by flipping config at both ends, using one
  audited implementation instead of a per-product sidecar; the `tls` profile field becomes
  deployment config, not a per-product dialect.
- Depends on: phase6_canonical-behavior-spec (SPEC-008 TLS section). Independent of the handshake and
  cheap-convergence tasks — can proceed in parallel with them once the spec lands.
