import js from "@eslint/js";
import ts from "typescript-eslint";
import hooks from "eslint-plugin-react-hooks";
import jsxA11y from "eslint-plugin-jsx-a11y";
import globals from "globals";

// Design rules (DESIGN §18.7 rules 1, 2 and 4, #455). They look inside `style={{…}}` only: a stylesheet is stylelint's
// (#454). Computed values (`width: `${ch}ch``, `var(--t3p)`, `...tokens`) pass; a literal colour or px does not.
const STYLE = "JSXAttribute[name.name='style']";
// Properties whose value is a colour, and those whose bare number React turns into px.
const COLOUR_PROPS =
  "/^(color|background.*|border|border.*Color|outline|outlineColor|fill|stroke|boxShadow|textShadow|caretColor|accentColor|textDecorationColor)$/";
const PX_PROPS =
  "/^(width|height|min.*|max.*|margin.*|padding.*|gap|rowGap|columnGap|top|left|right|bottom|inset.*|fontSize|border.*(Width|Radius)|flexBasis|letterSpacing|outlineOffset)$/";
// A string that is not `var(--token)` or a CSS-wide / paint keyword.
const NOT_A_TOKEN =
  "/^(?!(var\\(--[a-z0-9-]+\\)|inherit|initial|unset|currentcolor|transparent|none|auto)$)/i";
const COLOUR_LITERAL =
  "/^#[0-9a-f]{3,8}$|^(rgb|hsl|hwb|lab|lch|oklab|oklch|color)a?\\(/i";
const SVG = "JSXOpeningElement[name.name='svg']";

const COLOUR_MSG =
  "Colours in inline styles are tokens: use var(--token) (DESIGN §18.7 rule 1).";
const PX_MSG =
  "Sizes in inline styles are tokens or computed values, never a literal px (DESIGN §18.7 rule 2).";

const DESIGN_SYNTAX = [
  [`${STYLE} Literal[value=${COLOUR_LITERAL}]`, COLOUR_MSG],
  [
    `${STYLE} Property[key.name=${COLOUR_PROPS}] > Literal[value=${NOT_A_TOKEN}]`,
    COLOUR_MSG,
  ],
  [`${STYLE} Literal[value=/\\d\\s*px/]`, PX_MSG],
  [`${STYLE} TemplateElement[value.raw=/(^|\\d)px/]`, PX_MSG],
  [
    `${STYLE} Property[key.name=${PX_PROPS}] > Literal[raw=/^[1-9]|^0?\\.\\d*[1-9]/]`,
    PX_MSG,
  ],
  [
    SVG,
    "No raw <svg> outside src/icons/: icons are Lucide (DESIGN §18.7 rule 4). A data graphic needs a reasoned eslint-disable.",
  ],
].map(([selector, message]) => ({ selector, message }));

// Any other icon set; lucide-react is the one the app bundles.
const ICON_PACKAGES =
  "^(react-icons|@heroicons|@fortawesome|feather-icons|react-feather|@tabler|@phosphor-icons|@mui/icons-material|@radix-ui/react-icons|@iconify|@ant-design/icons|bootstrap-icons|ionicons|@primer/octicons|lucide$|lucide-(?!react$))";

export default ts.config(
  { ignores: ["dist/**", "src/bindings.ts", "src-tauri/**"] },
  js.configs.recommended,
  ...ts.configs.recommended,
  {
    // Tool configs (stylelint.config.mjs) run in Node.
    files: ["*.mjs"],
    languageOptions: { globals: globals.node },
  },
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
  // Accessibility (DESIGN §18.7 rule 7); axe in the harness (#457) covers what a linter cannot see.
  { ...jsxA11y.flatConfigs.recommended, files: ["src/**/*.tsx"] },
  {
    // `no-restricted-syntax` is one rule: a later block that sets it replaces this list, so src/stream spreads
    // DESIGN_SYNTAX in and src/icons filters the <svg> ban out.
    files: ["src/**/*.{ts,tsx}"],
    rules: {
      "no-restricted-syntax": ["error", ...DESIGN_SYNTAX],
      "no-restricted-imports": [
        "error",
        {
          patterns: [
            {
              regex: ICON_PACKAGES,
              message:
                "Icons come from lucide-react only (DESIGN §18.1, §18.7 rule 4).",
            },
          ],
        },
      ],
    },
  },
  {
    // The one place hand-drawn SVG may live (none today).
    files: ["src/icons/**/*.tsx"],
    rules: {
      "no-restricted-syntax": [
        "error",
        ...DESIGN_SYNTAX.filter((r) => r.selector !== SVG),
      ],
    },
  },
  {
    // Everything off the channel is plain JSON that the stream client replaces, never mutates; cloning it hides a
    // reactive proxy or a shared reference instead of fixing it (ADR-0001 §4.3, #89).
    files: ["src/stream/**/*.{ts,tsx}"],
    rules: {
      "no-restricted-syntax": [
        "error",
        ...DESIGN_SYNTAX,
        // `structuredClone(x)`, `window.structuredClone(x)` and `globalThis.structuredClone(x)` all name the identifier;
        // the computed forms `globalThis["structuredClone"]` and a template literal name it as a string.
        ...[
          "Identifier[name='structuredClone']",
          "Literal[value='structuredClone']",
          "TemplateLiteral[quasis.length=1][quasis.0.value.cooked='structuredClone']",
        ].map((selector) => ({
          selector,
          message:
            "structuredClone is forbidden in src/stream: stream data is plain JSON, replace it, do not clone it.",
        })),
      ],
    },
  },
);
