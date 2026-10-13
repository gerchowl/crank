import js from "@eslint/js";
import svelte from "eslint-plugin-svelte";
import globals from "globals";
import ts from "typescript-eslint";
import svelteConfig from "./svelte.config.js";

export default ts.config(
  { ignores: ["dist/", "shots/", "node_modules/", "**/*.tmp.mjs"] },
  js.configs.recommended,
  ...ts.configs.strictTypeChecked,
  ...svelte.configs.recommended,
  {
    languageOptions: {
      globals: { ...globals.browser, ...globals.node },
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
        extraFileExtensions: [".svelte"],
      },
    },
  },
  {
    files: ["**/*.svelte", "**/*.svelte.ts"],
    languageOptions: {
      parserOptions: { parser: ts.parser, svelteConfig },
    },
  },
  {
    rules: {
      "@typescript-eslint/no-explicit-any": "error",
      // `onclick={() => doThing()}` is the idiomatic Svelte handler shape
      "@typescript-eslint/no-confusing-void-expression": ["error", { ignoreArrowShorthand: true }],
      "@typescript-eslint/restrict-template-expressions": ["error", { allowNumber: true }],
      "@typescript-eslint/no-unused-vars": ["error", { argsIgnorePattern: "^_", varsIgnorePattern: "^_" }],
      "svelte/no-unused-svelte-ignore": "error",
      "svelte/valid-compile": "error",
      "svelte/require-each-key": "error",
      "svelte/no-reactive-reassign": "error",
      "svelte/prefer-svelte-reactivity": "error",
      "svelte/no-at-html-tags": "error",
      "svelte/button-has-type": "error",
      "svelte/no-useless-mustaches": "error",
      "svelte/prefer-writable-derived": "error",
    },
  },
  {
    files: ["**/*.svelte"],
    rules: {
      // `{@render snippet()}` is typed as a void call by the parser → false positives in markup
      "@typescript-eslint/no-confusing-void-expression": "off",
    },
  },
  {
    files: ["*.config.js", "*.config.ts", "scripts/*.mjs"],
    ...ts.configs.disableTypeChecked,
  },
);
