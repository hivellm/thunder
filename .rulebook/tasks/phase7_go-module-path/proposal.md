# Proposal: phase7_go-module-path (GH #9, filed by the Synap Go SDK)

## Why
The README lists the Go client as `github.com/hivellm/thunder-go`,
"implemented and tested". **It cannot be fetched** — that repository does not
exist:

```
$ go list -m github.com/hivellm/thunder-go@latest
go: module github.com/hivellm/thunder-go: ... exit status 128
$ curl -s -o /dev/null -w "%{http_code}" https://github.com/hivellm/thunder-go
404
```

The code lives in `go/` inside this monorepo while `go/go.mod` declares a
module path that corresponds to no repository. Go resolves a module by fetching
the repo at its declared path, so **the declared path can never resolve while
the code lives here**.

So the Go lane is documented as consumable and is not. Synap's Go SDK keeps its
own hand-written transport as a result — the one Synap SDK still carrying a
private copy of the protocol, which is precisely what Thunder exists to end.

## What Changes
Option 2 from the issue, and the issue argues for it correctly: **keep the code
in the monorepo and fix the path**.

- `go/go.mod` becomes `module github.com/hivellm/thunder/go`
- releases are tagged `go/v0.2.0`, which is Go's native subdirectory-module
  convention
- consumers write `go get github.com/hivellm/thunder/go@v0.2.0`

Option 1 (split the repo to match the current path) is rejected: it adds a
second repository to keep in step with the release train, and GH #8 shows
keeping *one* train aligned is already the hard part. Splitting would add a
place for exactly the drift we just got bitten by.

## Impact
- Governing spec: SPEC-006 (PKG-050, the Go lane)
- Affected code: go/go.mod, every internal import path in `go/`, the root
  README's Packages table, the release workflow's tagging step
- Breaking change: **for Go consumers, yes** — the import path changes. Nobody
  can be consuming it today, since the current path 404s, so the practical
  blast radius is zero
- User benefit: the Go client becomes consumable at all, and rides the same
  tags as everything else instead of needing its own release choreography
