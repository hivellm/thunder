# Proposal: phase0_names-spec-profiles

## Why
Package names must be frozen before any code references them (G0), the normative wire spec must move into this repo so SPEC-001 has something to bind to, and the profile dimensions must exist as data before client/server enforcement is built (DAG T0.2 + T0.3).

## What Changes
Reserve/confirm registry names (crates.io `thunder-wire`/`thunder-client`/`thunder-server`; npm `@hivellm/thunder` — decide `@hivellm` vs `@hivehub` org; PyPI `hivellm-thunder`; NuGet `HiveLLM.Thunder`), recording fallbacks if taken. Transplant `Nexus/docs/specs/rpc-wire-format.md` v1 verbatim into `docs/spec/` with a provenance header. Author `conformance/profiles/{synap,nexus,vectorizer,lexum}.yaml` with the SPEC-002 dimensions and PRO-011 values. Verify the Vectorizer-TLS ordering question (SPEC-004 SRV-040 note) and record the decision.

## Impact
- Governing specs: SPEC-006 (PKG-010), SPEC-001 §1 (binding target), SPEC-002 (PRO-001, PRO-010/011)
- PRD requirements: FR-60, FR-01, FR-10, FR-11
- DAG: T0.2, T0.3 (gate G0)
- Affected code: docs/spec/ (new), conformance/profiles/ (new); no product code
- Breaking change: NO
- User benefit: one normative spec home; names and profiles stable for every downstream task
