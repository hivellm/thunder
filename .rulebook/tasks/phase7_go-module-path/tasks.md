## 1. Implementation
- [ ] 1.1 Change `go/go.mod` to `module github.com/hivellm/thunder/go`
- [ ] 1.2 Update every internal import in `go/` (and the interop/conformance harnesses if they reference the module path)
- [ ] 1.3 Verify the module resolves the way a consumer would reach it — a subdirectory module needs the `go/vX.Y.Z` tag form, so confirm the tag shape is right rather than assuming
- [ ] 1.4 Add the Go tagging step to the release workflow so `go/v0.2.0` is pushed alongside `v0.2.0`, keeping the single release train
- [ ] 1.5 Fix the root README's Packages table — it currently documents an import path that fails

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation — `go/README.md` and the root README with the resolvable path and the `@v` form a consumer types
- [ ] 2.2 Write tests covering the new behavior — the Go test suite still passes under the new module path, and the conformance corpus run is unaffected
- [ ] 2.3 Run tests and confirm they pass — Go gate green (gofmt, vet, test), and ideally a real `go get` of the tagged path from outside the repo, since resolution is the whole point of this task
