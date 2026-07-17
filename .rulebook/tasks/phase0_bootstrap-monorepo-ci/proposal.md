# Proposal: phase0_bootstrap-monorepo-ci

## Why
Thunder is an empty repo with docs only. Every later task needs the monorepo skeleton, workspace lints, and the 3-OS CI matrix in place first — G0 blocks everything (DAG T0.1).

## What Changes
Create the monorepo layout (`rust/`, `typescript/`, `python/`, `csharp/`, `conformance/`) with skeleton packages that compile empty, family workspace lints (clippy `-D warnings`, `unwrap_used` denied), and CI lanes: Rust fmt+clippy+test on Linux/macOS/Windows; tsc+eslint+vitest; ruff+pytest; `dotnet build -warnaserror`+test. Quality-gate order per family convention: type-check → lint → tests.

## Impact
- Governing spec: SPEC-006 §1 (PKG-001, PKG-002) — docs/specs/SPEC-006-packaging-release.md
- PRD requirements: NFR-08
- DAG: T0.1 (blocks G0 and everything downstream)
- Affected code: new — repo root, CI workflows, skeleton crates/packages
- Breaking change: NO (green-field)
- User benefit: every subsequent task lands on a linted, cross-platform CI floor
