# Proposal: phase7_release-version-guard (GH #8, filed by the Synap SDK swap)

## Why
NuGet sat at 0.1.0 while crates.io, npm and PyPI were at 0.2.0. **Nothing
noticed.** The README's central promise is "one release train, one version",
and a consumer could not honour it: Synap's Rust, TypeScript and Python SDKs
pinned 0.2.0 while its C# SDK could only reach 0.1.0.

That was not cosmetic. A 0.1.0 C# client is missing the GH #6 work, so it
treats a zero-length frame as a parse failure and poisons the connection where
a 0.2.0 client treats it as the keep-alive WIRE-024 defines. **Two Thunder
clients disagreeing about the same server** is exactly what Thunder exists to
prevent.

**The publish itself is now done** — NuGet has 0.2.0, all four registries are
aligned. What is still missing is the thing that would have caught it: nothing
in CI compares what is published against what the repo claims.

This has now happened twice. npm silently lagged when its publish job was
removed for the OTP requirement (3ac8a49), and NuGet lagged on a rejected API
key. Both were found by a person, late, from outside.

## What Changes
A CI check that fails when any registry's latest published version differs from
the version in the repo, so the gap is loud and immediate rather than
discovered by a consumer months later.

Design points worth getting right rather than rushing:

- It must **not** fail on the normal state of an unreleased commit (repo ahead
  of every registry is the usual case between releases). The check is about
  *drift between registries*, or a registry lagging a **tagged** release.
- It should name the lagging registry and both versions, so the fix is obvious
  from the failure text.
- Go has no registry to query — it resolves from the VCS tag — so it needs
  either its own check or an explicit exemption (see phase7_go-module-path).

## Impact
- Governing spec: SPEC-006 (PKG-011, one release train)
- Affected code: .github/workflows/ (new or extended job)
- Breaking change: **NO** — CI only
- User benefit: the one-version promise becomes enforced rather than asserted;
  the next lagging registry is caught the day it lags
