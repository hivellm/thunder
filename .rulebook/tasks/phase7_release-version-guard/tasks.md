## 1. Implementation
- [ ] 1.1 Decide **when** the check runs and what it compares. Proposal: on a `v*` tag it demands every registry match the tag; on a normal push it only fails when two registries disagree with *each other* (drift), never when the repo is simply ahead of all of them. Record the choice — a check that cries wolf on every unreleased commit gets ignored, which is worse than no check
- [ ] 1.2 Query the four registries: crates.io (`/api/v1/crates/thunder-rpc`, **requires a User-Agent header** — without it the request fails and a naive parse reads as "nothing published"), npm registry, PyPI JSON API, NuGet flat-container index
- [ ] 1.3 Compare against the repo manifests, reusing the same extraction the existing tag-vs-manifest step uses so the two cannot disagree
- [ ] 1.4 Fail with a message naming the registry, the published version and the repo version
- [ ] 1.5 Handle the Go lane explicitly — it publishes from a VCS tag with no registry to query, so either check the tag or state the exemption in the workflow rather than leaving it silently uncovered

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation — SPEC-006 PKG-011 note that the one-version rule is now enforced by CI, and how
- [ ] 2.2 Write tests covering the new behavior — exercise the comparison logic against fixed inputs (all aligned / one lagging / repo ahead of all) rather than only against the live registries, so the check itself is testable offline and a network blip does not read as a failure
- [ ] 2.3 Run tests and confirm they pass — and confirm the check is green against the current state, where all four registries are at 0.2.0
