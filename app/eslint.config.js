import js from "@eslint/js";
import ts from "typescript-eslint";
import hooks from "eslint-plugin-react-hooks";
import jsxA11y from "eslint-plugin-jsx-a11y";
import globals from "globals";

// Design rules (DESIGN §18.7 rules 1, 2 and 4, #455). They look inside a `style={{…}}` literal only: a stylesheet is
// stylelint's (#454), and a style object built elsewhere (`const s = {…}; <div style={s}>`) is invisible to a selector.
// Computed values (`width: `${ch}ch``, `var(--t3p)`, `...tokens`) pass; a literal colour or px does not.
const STYLE = "JSXAttribute[name.name='style']";
const NAMED_COLOURS = [
  ..."aliceblue antiquewhite aqua aquamarine azure beige bisque black blanchedalmond blue blueviolet brown burlywood cadetblue chartreuse chocolate coral cornflowerblue cornsilk crimson cyan darkblue darkcyan darkgoldenrod darkgray darkgreen darkgrey darkkhaki darkmagenta darkolivegreen darkorange darkorchid darkred darksalmon darkseagreen darkslateblue darkslategray darkslategrey darkturquoise darkviolet deeppink deepskyblue dimgray dimgrey dodgerblue firebrick floralwhite forestgreen fuchsia gainsboro ghostwhite gold goldenrod gray green greenyellow grey honeydew hotpink indianred indigo ivory khaki lavender lavenderblush lawngreen lemonchiffon lightblue lightcoral lightcyan lightgoldenrodyellow lightgray lightgreen lightgrey lightpink lightsalmon lightseagreen lightskyblue lightslategray lightslategrey lightsteelblue lightyellow lime limegreen linen magenta maroon mediumaquamarine mediumblue mediumorchid mediumpurple mediumseagreen mediumslateblue mediumspringgreen mediumturquoise mediumvioletred midnightblue mintcream mistyrose moccasin navajowhite navy oldlace olive olivedrab orange orangered orchid palegoldenrod palegreen paleturquoise palevioletred papayawhip peachpuff peru pink plum powderblue purple rebeccapurple red rosybrown royalblue saddlebrown salmon sandybrown seagreen seashell sienna silver skyblue slateblue slategray slategrey snow springgreen steelblue tan teal thistle tomato turquoise violet wheat white whitesmoke yellow yellowgreen".split(
    " ",
  ),
];
// A hex colour, a colour function or a named colour anywhere in a string or template text. The look-arounds keep
// `var(--red)` and `redundant` out. (`transparent`, `currentcolor` and the CSS-wide keywords are not colour literals.)
const COLOUR_LITERAL = `/(?<![\\w&-])(#[0-9a-f]{3,8}(?![\\w-])|(rgb|hsl|hwb|lab|lch|oklab|oklch|color|color-mix)a?\\(|(${NAMED_COLOURS.join("|")})(?![\\w-]))/i`;
// Unitless numbers React keeps unitless, plus custom properties (a runtime token such as `--ch`).
const UNITLESS =
  "/^(opacity|zIndex|flex|flexGrow|flexShrink|order|fontWeight|lineHeight|aspectRatio|--.*)$/";
// A number other than 0: React turns it into px on every property outside UNITLESS.
const NONZERO = "/^[+-]?(?!0*\\.?0*$)[0-9.]/";
const NOT_UNITLESS = `:not([key.name=${UNITLESS}]):not([key.value=${UNITLESS}])`;
const SVG_SELECTORS = [
  "JSXOpeningElement[name.name='svg']",
  "CallExpression[callee.name='createElement'] > Literal[value='svg']",
  "CallExpression[callee.property.name='createElement'] > Literal[value='svg']",
  "JSXAttribute[name.name='dangerouslySetInnerHTML'] Literal[value=/<svg/i]",
  "JSXAttribute[name.name='dangerouslySetInnerHTML'] TemplateElement[value.raw=/<svg/i]",
];

const COLOUR_MSG =
  "Colours in inline styles are tokens: use var(--token) (DESIGN §18.7 rule 1).";
const PX_MSG =
  "Sizes in inline styles are tokens or computed values, never a literal px or bare number (DESIGN §18.7 rule 2).";
const SVG_MSG =
  "No raw <svg> outside src/icons/: icons are Lucide (DESIGN §18.7 rule 4). A data graphic needs a reasoned eslint-disable.";

const DESIGN_SYNTAX = [
  [`${STYLE} Literal[value=${COLOUR_LITERAL}]`, COLOUR_MSG],
  [`${STYLE} TemplateElement[value.raw=${COLOUR_LITERAL}]`, COLOUR_MSG],
  [`${STYLE} Literal[value=/\\d\\s*px/]`, PX_MSG],
  [`${STYLE} TemplateElement[value.raw=/(^|\\d)\\s*px/]`, PX_MSG],
  // `margin: 8`, `margin: -8`, `a ? 8 : 4`; `0` and the UNITLESS properties pass.
  [`${STYLE} Property${NOT_UNITLESS} > Literal[raw=${NONZERO}]`, PX_MSG],
  [
    `${STYLE} Property${NOT_UNITLESS} > UnaryExpression > Literal[raw=${NONZERO}]`,
    PX_MSG,
  ],
  [
    `${STYLE} Property${NOT_UNITLESS} > ConditionalExpression > Literal[raw=${NONZERO}]`,
    PX_MSG,
  ],
  ...SVG_SELECTORS.map((selector) => [selector, SVG_MSG]),
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
    // The design harness (#457): the spec and config run in Node; the audit and the fixture run in the page.
    files: ["e2e/design/**/*.{js,ts,tsx}", "playwright.config.ts"],
    languageOptions: { globals: { ...globals.node, ...globals.browser } },
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
        ...DESIGN_SYNTAX.filter((r) => !SVG_SELECTORS.includes(r.selector)),
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
