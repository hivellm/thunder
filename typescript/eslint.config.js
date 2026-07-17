// Flat ESLint config: typescript-eslint's type-checked recommended set
// over the whole package (src, tests, config files in the tsconfig
// project). The `any`-hygiene rules keep `any` out of the public API.
import { fileURLToPath } from "node:url";

import tseslint from "typescript-eslint";

const tsconfigRootDir = fileURLToPath(new URL(".", import.meta.url));

export default tseslint.config(
  {
    ignores: ["dist/", "coverage/", "node_modules/"],
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
