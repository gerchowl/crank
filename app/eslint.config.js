import js from "@eslint/js";
import ts from "typescript-eslint";
import hooks from "eslint-plugin-react-hooks";
import globals from "globals";
export default ts.config(
  { ignores: ["dist/**", "src/bindings.ts", "src-tauri/**"] },
  js.configs.recommended,
  ...ts.configs.recommended,
  {
    // e2e specs run in Node and hand functions to the WebView, so both global sets apply (#35).
    files: ["e2e/**/*.mjs"],
    languageOptions: {
      globals: { ...globals.node, ...globals.browser, ...globals.mocha },
    },
  },
  {
    files: ["src/**/*.{ts,tsx}"],
    languageOptions: { globals: globals.browser },
    plugins: { "react-hooks": hooks },
    rules: hooks.configs.recommended.rules,
  },
  {
    // Everything off the channel is plain JSON that the stream client replaces, never mutates; cloning it hides a
    // reactive proxy or a shared reference instead of fixing it (ADR-0001 §4.3, #89).
    files: ["src/stream/**/*.{ts,tsx}"],
    rules: {
      "no-restricted-syntax": [
        "error",
        {
          selector: "Identifier[name='structuredClone']",
          message:
            "structuredClone is forbidden in src/stream: stream data is plain JSON, replace it, do not clone it.",
        },
      ],
    },
  },
);
