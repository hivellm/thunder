# Proposal: phase6_agnostic-config

## Why
Owner directive (2026-07-17): *"por mais que o projeto tenha nascido com base nas implementações de RPC
dos projetos ele precisa ser totalmente agnóstico e configurável para implementações futuras, portanto
nada de profiles nominais"* and *"não quero profiles diferentes específicos por projeto, quero
configurações que cada aplicação possa definir, de preferência todas usando um padrão."*

Thunder today is the opposite. SPEC-002 is titled "Profiles" and exists to catalogue products:
`Profile::synap()` / `nexus()` / `vectorizer()` / `lexum()` are compiled into the library, `registry()`
enumerates them, and `conformance/profiles/*.yaml` pins them by product name. Product knowledge is baked
into the protocol library — a library that must serve implementations that do not exist yet cannot ship
a hardcoded list of four products from 2026. The named registry also inverts ownership: a new product
waits on a Thunder release to get a row, and Thunder carries a maintenance burden for behavior it does
not own.

This is the end state the behavioral-normalization analysis identified (BN-013/BN-020 — "the registry
that today encodes their differences ends up encoding only their names"), reached directly by design
rather than through a four-phase migration: if the names never ship, there is nothing to converge.

## What Changes
Thunder ships **one standard configuration** and **zero product knowledge**:
- **Delete every named constant and the registry**: `Profile::synap()`, `nexus()`, `vectorizer()`,
  `lexum()`, `registry()`, and `conformance/profiles/{synap,nexus,vectorizer,lexum}.yaml`. Nothing in
  Thunder names a product again.
- **Rename `Profile` → `Config`** across all four languages. "Profile" in this codebase means "a
  product's row in the registry" — the exact concept being deleted; keeping the word would preserve the
  model the directive rejects. The type was always a plain settings struct; now the name says so.
- **One standard**: `Config::standard()` (also `Default`) carries the canonical behavior — mandatory
  HELLO map with `proto` negotiation + capabilities reply (the only handshake that can evolve, which is
  what "future implementations" needs), `[CODE]` error superset, 64 MiB cap, 256 in-flight, push
  reserved, TLS off. Identity (`scheme`, `port`) has no default: the application supplies it.
- **Everything is a knob**: a builder on `Config` for every dimension, so an application that diverges
  (Synap's `AUTH`-no-HELLO, Nexus's arg-less HELLO, Vectorizer's mandatory HELLO) expresses that **in
  its own repository**, not in Thunder. Convergence becomes "delete overrides until only scheme+port
  remain" — visible, per-app, with no coordinated Thunder release.
- **Conformance keeps its guarantee without names**: one `conformance/standard.yaml` pins the standard's
  defaults, and every language's `Config::standard()` is pinned to it — so the four languages can still
  never disagree, which was the registry's only legitimate job.
- The verified per-product facts (BN-023) stay documented in `docs/analysis/` as the reference the owner
  uses when configuring each application — knowledge, not shipped code.

## Impact
- Affected specs: SPEC-002 (rewritten — no longer a product registry; PRO-010/PRO-011 registry
  requirements deleted), SPEC-003 (CLT-002/050 reference the config, not a profile), SPEC-004 (SRV-011),
  SPEC-005 (TST — profile pinning becomes standard pinning)
- Affected code: `rust/thunder/src/wire/profile.rs` → `config.rs` + every reference; the three language
  packages' profile modules + pinning tests; `rust/thunder-bench` (its `bench_profile()` is already a
  custom config — it becomes the worked example of an app defining its own); `conformance/profiles/`
- Breaking change: YES for the pre-1.0 API surface (the named constants disappear; `Profile` → `Config`).
  NO on the wire — bytes are untouched (PRO-003 still holds: config is data, never behavior).
- User benefit: Thunder becomes a protocol library instead of a product catalogue. A future
  implementation configures itself with no Thunder change; the family converges by deleting overrides;
  nobody waits on a release for a row in a registry.
