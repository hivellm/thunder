// Flat ESLint config: typescript-eslint's type-checked recommended set
// over the whole package (src, tests, config files in the tsconfig
// project). The `any`-hygiene rules keep `any` out of the public API.
import { fileURLToPath } from "node:url";

import tseslint from "typescript-eslint";

const tsconfigRootDir = fileURLToPath(new URL(".", import.meta.url));

export default tseslint.config(
  {
    // dist/coverage/node_modules are build output; interop-probe.ts is a
    // standalone tsx harness script (run by interop/run.py), not part of the
    // tsconfig project or the published library, so the type-checked parser
    // cannot resolve it — exclude it like the other non-project files.
    ignores: ["dist/", "coverage/", "node_modules/", "interop-probe.ts"],
  },
  {
    files: ["**/*.ts"],
    extends: [...tseslint.configs.recommendedTypeChecked],
    languageOptions: {
      parserOptions: {
        projectService: true,
        tsconfigRootDir,
      },
    },
    rules: {
      "@typescript-eslint/no-explicit-any": "error",
      "@typescript-eslint/no-unused-vars": [
        "error",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_" },
      ],
      "@typescript-eslint/explicit-module-boundary-types": "error",
    },
  },
);
