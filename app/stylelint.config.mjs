// DESIGN §18.7 rules 1 to 3 (#454): colours and sizes come from the theme.css tokens, and every var() resolves.
// Rules start strict; an exception is listed here with its reason, never silenced in a stylesheet.

// Custom properties set at runtime on <html> by useTypeTiers (typeTiers.ts `FALLBACK_TOKENS`), so no stylesheet
// declares them. stylelintConfig.test.ts pins this list to that object.
export const RUNTIME_TOKENS = [
  "--t1",
  "--t1r",
  "--t2",
  "--t3p",
  "--t3",
  "--t4",
];

const KEYWORDS = [
  "inherit",
  "initial",
  "unset",
  "revert",
  "currentColor",
  "transparent",
  "none",
  "auto",
  "normal",
  "0",
];

const STROKE = [
  ...KEYWORDS,
  "/^\\d+px$/",
  "solid",
  "dashed",
  "dotted",
  "inset",
];

const TOKEN_PROPERTIES = [
  "color",
  "background-color",
  "border-color",
  "gap",
  "row-gap",
  "column-gap",
  "/^margin/",
  "/^padding/",
  "font-size",
  "font-family",
  "border-radius",
  "height",
  "min-height",
  "line-height",
  // Shorthands: expandShorthand checks the colour, size and family inside them (border, background, font, ...).
  "background",
  "border",
  "/^border-(top|right|bottom|left)$/",
  "font",
  "outline",
  "box-shadow",
  "text-shadow",
  "inset",
  "/^border-(top|bottom)-(left|right)-radius$/",
];

export default {
  plugins: [
    "stylelint-declaration-strict-value",
    "stylelint-value-no-unknown-custom-properties",
  ],
  rules: {
    "color-no-hex": true,
    // Named colours and colour functions are literals too (rule 1); `currentColor` and `transparent` stay keywords.
    "color-named": "never",
    "function-disallowed-list": [
      "/^(rgb|rgba|hsl|hsla|hwb|lab|lch|oklab|oklch|color)$/",
    ],
    // Rule 8: a control must keep a visible focus ring. `outline: none` and `outline: 0` remove it; draw a
    // replacement (`outline-color`, `box-shadow`) instead of switching the outline off.
    "declaration-property-value-disallowed-list": {
      "/^outline(-style|-width)?$/": ["none", "0"],
    },
    "scale-unlimited/declaration-strict-value": [
      TOKEN_PROPERTIES,
      {
        // var() passes (ignoreVariables, the default); calc(), clamp() and colour functions do not.
        ignoreFunctions: false,
        expandShorthand: true,
        // Keywords, plus the unitless `0` (no margin, no padding, no gap), and three exceptions that are not
        // sizes on a scale. Each is listed here with its reason, never silenced in a stylesheet.
        ignoreValues: {
          "": KEYWORDS,
          // A unitless line-height is a ratio of the element's own font size, not a length.
          "line-height": [...KEYWORDS, "/^\\d+(\\.\\d+)?$/"],
          // Fill the parent or the viewport: relational, not a length on the scale. `height` also takes the
          // 1px box of the visually-hidden idiom (.sr-only).
          height: [...KEYWORDS, "100%", "100vh", "1px"],
          "min-height": [...KEYWORDS, "100%", "100vh"],
          // Stroke widths, line styles and shadow geometry are not on the size scales; only the colour inside
          // these shorthands must be a token (it is a var(), or the keyword/colour rules above catch it).
          border: [...STROKE],
          "/^border-(top|right|bottom|left)$/": [...STROKE],
          outline: [...STROKE],
          "box-shadow": [...STROKE],
          "text-shadow": [...STROKE],
          // A numeric font-weight is not on a scale either.
          font: [
            ...KEYWORDS,
            "/^var\\(--[\\w-]+\\)\\/\\d+(\\.\\d+)?$/",
            "/^[1-9]00$/",
            "bold",
            "bolder",
            "lighter",
          ],
        },
        message:
          "Use a theme.css token for ${property} (var(--…)); got ${value} (DESIGN §18.7 rules 1 and 2).",
      },
    ],
    "csstools/value-no-unknown-custom-properties": [
      true,
      {
        importFrom: [
          new URL("./src/theme.css", import.meta.url).pathname,
          {
            customProperties: Object.fromEntries(
              RUNTIME_TOKENS.map((t) => [t, ""]),
            ),
          },
        ],
      },
    ],
  },
  overrides: [
    {
      // The one file that may hold literals: it defines the tokens.
      files: ["src/theme.css"],
      rules: {
        "color-no-hex": null,
        "color-named": null,
        "function-disallowed-list": null,
        "scale-unlimited/declaration-strict-value": null,
      },
    },
  ],
};
