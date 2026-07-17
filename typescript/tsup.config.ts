import { defineConfig } from "tsup";

/** Dual ESM + CJS build with bundled type declarations (PKG-010). */
export default defineConfig({
  entry: ["src/index.ts"],
  format: ["esm", "cjs"],
  dts: true,
  sourcemap: true,
  clean: true,
  platform: "node",
  target: "node18",
});
