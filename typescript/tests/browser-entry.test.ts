/**
 * The wire entry must stay free of Node builtins (GH #10).
 *
 * The failure this guards against is a *resolution* error at bundle time, not
 * a runtime one: the main entry statically imports `node:fs`, `node:net` and
 * `node:tls` through the client, so a browser bundler fails on Thunder even
 * when the consumer touches none of that. A test that merely imports the
 * module under Node would not exercise the failure at all — it has to inspect
 * what the built artifact actually imports.
 *
 * So this reads `dist/` rather than `src/`. It is skipped when the package has
 * not been built, so `npm test` on a fresh clone stays green; CI builds before
 * testing, and the release gate builds too.
 */

import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

const dist = join(dirname(fileURLToPath(import.meta.url)), "..", "dist");
const built = existsSync(join(dist, "wire.js"));

/** Any import of a Node builtin, with or without the `node:` prefix. */
const NODE_BUILTIN =
  /(?:from\s*|require\()\s*["'](?:node:)?(fs|net|tls|http|https|stream|crypto|path|os|child_process)["']/;

describe.skipIf(!built)("the wire entry is bundler-safe for the browser", () => {
  it("imports no Node builtin, in either module format", () => {
    for (const file of ["wire.js", "wire.cjs"]) {
      const source = readFileSync(join(dist, file), "utf8");
      const offender = source.match(NODE_BUILTIN);
      expect(
        offender?.[0],
        `${file} must not import a Node builtin — that is exactly what breaks a browser bundle`,
      ).toBeUndefined();
    }
  });

  it("still exports what a browser consumer needs", () => {
    const source = readFileSync(join(dist, "wire.js"), "utf8");
    for (const name of ["FrameReader", "encodeRequest", "decodeResponseBody", "Value"]) {
      expect(source, `wire entry must export ${name}`).toContain(name);
    }
  });

  it("does not drag the client in", () => {
    const source = readFileSync(join(dist, "wire.js"), "utf8");
    // `Client` is the Node-only surface; its presence would mean the entry is
    // pulling the transport, and the builtins would follow. The bundler emits
    // `var Client = class …`, not `class Client`, so match the declaration the
    // build actually produces.
    expect(source).not.toMatch(/\bvar Client = class\b/);
  });

  it("leaves the main entry alone — Node consumers keep the client", () => {
    const source = readFileSync(join(dist, "index.js"), "utf8");
    expect(source).toMatch(NODE_BUILTIN);
    expect(source).toMatch(/\bvar Client = class\b/);
  });
});
