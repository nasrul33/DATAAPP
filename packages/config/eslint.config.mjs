import eslint from "@eslint/js";
import tseslint from "typescript-eslint";

export function createTerataiEslintConfig(rootDirectory) {
  const typedFiles = ["**/*.ts", "**/*.tsx"];
  const scopeToTypeScript = (config) => ({ ...config, files: typedFiles });

  return tseslint.config(
    {
      ignores: [
        "**/.venv/**",
        "**/coverage/**",
        "**/dist/**",
        "**/node_modules/**",
        "**/target/**",
        "**/*.tsbuildinfo",
      ],
    },
    eslint.configs.recommended,
    ...tseslint.configs.strictTypeChecked.map(scopeToTypeScript),
    ...tseslint.configs.stylisticTypeChecked.map(scopeToTypeScript),
    {
      files: typedFiles,
      languageOptions: {
        parserOptions: {
          project: [
            "./tsconfig.test.json",
            "./apps/*/tsconfig.json",
            "./packages/*/tsconfig.json",
          ],
          tsconfigRootDir: rootDirectory,
        },
      },
      rules: {
        "@typescript-eslint/consistent-type-exports": "error",
        "@typescript-eslint/no-import-type-side-effects": "error",
      },
    },
  );
}
