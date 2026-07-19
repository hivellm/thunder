# Proposal: phase7_typescript-browser-build (GH #10, filed by Fluxum)

## Why
`@hivehub/thunder` cannot be bundled for a browser. Its single entry statically
imports Node builtins at the top level:

```js
import * as fs from "fs";
import * as net from "net";
import * as tls from "tls";
```

Any bundler targeting the browser fails to resolve those three, so **the build
breaks on Thunder rather than on anything the consumer did** — even when the
consumer touches none of the Node surface.

Fluxum's browser SDK uses exactly one thing from the package: `FrameReader`.
Its browser transport is `fetch` + `ReadableStream`; it never opens a socket,
never reads a file, never negotiates TLS. It still cannot build, and works
around it by aliasing all three to an empty module — a workaround every browser
consumer will now rediscover independently.

**This is not a size problem.** Fluxum reports that with the alias in place,
tree-shaking does drop the client: its entire browser bundle — SDK, Thunder's
frame codec, `@msgpack/msgpack`, and its own envelope layers — is 12.1 KB
min+gzip. The Node code genuinely disappears. The problem is that the package
advertises no browser story and fails with a resolution error from a dependency
instead of a clear "this entry is Node-only".

## What Changes
The issue offers two fixes; the second is nicer for consumers who want the
codec alone, and they compose:

1. A **`browser` export condition** pointing at a build with client/server
   stripped, so bundlers pick it automatically.
2. A **subpath export for the wire layer alone** (e.g. `@hivehub/thunder/wire`),
   so a consumer that only wants `FrameReader` and the codec says so explicitly
   and never pulls the transport at all.

(2) also matches how the Rust crate already works — `default-features = false`
gives the pure wire layer with no runtime — so it makes the two lanes
consistent rather than inventing a TypeScript-only idea.

## Impact
- Governing spec: SPEC-006 (packaging), SPEC-001 (the wire layer is pure by
  WIRE-030 — this makes the package reflect what the spec already says)
- Affected code: typescript/package.json (exports map), the build config, and
  possibly a source split so the wire layer has no transitive Node import
- Breaking change: **NO** — additive export conditions and a new subpath; the
  existing entry keeps working for Node consumers
- User benefit: browser consumers can use Thunder's codec at all, without
  discovering an aliasing workaround first
