# Proposal: phase6_canonical-behavior-spec

## Why
Thunder's feasibility work stops at "Profiles, not forks" — one codec + one client contract, with
per-product behavior parameterized as four profile rows. The behavioral-normalization analysis
(BN-001..BN-023) shows those rows can converge to a single canonical behavior, with the whole
program hinging on ONE decision the current specs don't record: the difference between a handshake's
**shape** (frame ordering, `proto` negotiation, capabilities reply) and auth **enforcement**
(whether credentials are required). Without that principle written down, "normalize the handshake"
reads as "make everyone require auth" — which would be a security regression for open deployments and
was never the goal. This task ratifies the canonical behavior as a normative spec so the
implementation tasks (TLS, handshake, cheap-convergence) build against one pinned definition instead
of re-litigating it each time. Per the owner's directive, the canonical HELLO handshake and TLS are
defined as **fully-implemented but optional** capabilities: shipped in the shared stack, opt-in per
product/deployment — never mandated.

## What Changes
A new normative spec `docs/specs/SPEC-008-canonical-behavior.md` pinning, for each behavioral
dimension, the single family behavior and its optionality:
- **Handshake** = the canonical shape is a leading `HELLO` map (`{version, client_name, optionally
  token | api_key | [user, pass]}`) with `proto` negotiation and a capabilities reply. It is a
  **capability every server can offer and every client can speak**, opt-in per profile — NOT a
  mandate. Record the **shape ≠ auth-policy** principle explicitly (auth enforcement stays a
  per-deployment toggle, like `require_auth`/`auth_required`). Legacy first-frames (no-HELLO+AUTH,
  optional arg-less HELLO) remain accepted via dual-accept.
- **TLS** = one optional, config/feature-gated `tokio-rustls` layer (no STARTTLS), **off by
  default**, offered by every profile — the family's first running RPC TLS. Optional but implemented.
- **Error grammar** = the `[CODE] message` superset with `NOAUTH`/`WRONGPASS`/`NOPERM`/`NOPROTO`
  recognized as codes; pin the exact spelling (bracketed codes vs bare tokens alongside `[CODE]`) —
  the one open decision from BN-011.
- **Caps** = 64 MiB configurable default, checked before allocation; a single configurable
  `max_in_flight` default.
- **Push** = `PUSH_ID` is server→client-only; every client ships the push hook; whether a server
  emits is a per-product capability, not a dialect.

Plus conformance corpus additions: canonical-HELLO request/reply vectors, canonical `[CODE]` error
vectors (incl. `NOPERM`), and **legacy-tolerance** vectors (decode/accept-only, never emitted) for
every deprecated form — no-HELLO connect, arg-less HELLO, bare `NOAUTH`/`WRONGPASS`, 512 MiB cap
config — mirroring the WIRE-011/016 tolerance discipline.

## Impact
- Affected specs: NEW docs/specs/SPEC-008-canonical-behavior.md; cross-refs into SPEC-001 (WIRE),
  SPEC-002 (PRO), SPEC-003 (CLT), SPEC-004 (SRV), SPEC-005 (TST)
- Affected code: conformance/vectors/ (new canonical + legacy-tolerance vectors); no product code in
  this task — it is decisions + fixtures that the implementation tasks consume
- Breaking change: NO (spec + additive corpus vectors only; wire bytes frozen)
- User benefit: one pinned definition of "the same way," with TLS and handshake explicitly optional;
  the shape ≠ policy principle prevents an accidental auth mandate; the corpus makes "converged"
  testable rather than a convention
- Depends on: phase6_registry-errata (the spec should describe the corrected registry, not the buggy
  one). Gates: phase6_tls-optional, phase6_handshake-optional, phase6_cheap-convergence.
