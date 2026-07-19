import { defineConfig } from "tsup";

/**
 * Two builds, not one — because they target different runtimes (PKG-010).
 *
 * - **`index`** — the whole package, Node-targeted. It reaches `node:fs`,
 *   `node:net` and `node:tls` through the client.
 * - **`wire`** — the wire layer alone, built `platform: "neutral"` so nothing
 *   Node-specific can slip in. The main entry's static Node imports are
 *   unresolvable in a browser bundler, so a consumer who wanted only the codec
 *   still had their build fail on Thunder (GH #10). This entry is the
 *   TypeScript counterpart of the Rust crate's `default-features = false`, and
 *   it is a *separate* config rather than a second entry in one config so the
 *   neutral platform genuinely applies to it.
 */
export default defineConfig([
  {
    entry: { index: "src/index.ts" },
    format: ["esm", "cjs"],
    dts: true,
    sourcemap: true,
    clean: true,
    platform: "node",
    target: "node18",
  },
  {
    entry: { wire: "src/wire-entry.ts" },
    format: ["esm", "cjs"],
    dts: true,
    sourcemap: true,
    // Must not clean: it would delete the index build above.
    clean: false,
    platform: "neutral",
    target: "es2022",
  },
]);
