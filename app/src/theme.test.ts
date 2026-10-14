import { readFileSync } from "node:fs";
import { describe, expect, test } from "vitest";
import { parseTheme } from "./theme";

// DESIGN §6.3: text >= 4.5:1, chart marks and UI boundaries >= 3:1, in both modes. Reads theme.css itself, so a
// token edit that breaks a pair fails here (the full palette validation is #30).
const css = readFileSync(new URL("./theme.css", import.meta.url), "utf8");
const block = (sel: string) => {
  const i = css.indexOf(sel);
  if (i < 0) throw new Error(`theme.css: ${sel} not found`);
  return css.slice(i, css.indexOf("}", i));
};
const tokens = (b: string) =>
  Object.fromEntries(
    [...b.matchAll(/--([\w-]+):\s*(#[0-9a-f]{6})/gi)].map((m) => [
      m[1]!,
      m[2]!,
    ]),
  );
const lum = (hex: string) => {
  const c = [1, 3, 5]
    .map((i) => parseInt(hex.slice(i, i + 2), 16) / 255)
    .map((v) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
  return 0.2126 * c[0]! + 0.7152 * c[1]! + 0.0722 * c[2]!;
};
const ratio = (a: string, b: string) => {
  const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p) as [number, number];
  return (x + 0.05) / (y + 0.05);
};
const PAIRS: [string, string, number][] = [
  ["ink-primary", "surface-0", 4.5],
  ["ink-primary", "surface-1", 4.5],
  ["ink-secondary", "surface-1", 4.5],
  ["ink-muted", "surface-1", 4.5],
  ["ink-muted", "surface-2", 4.5],
  ["accent-ink", "accent", 4.5],
  ["accent-text", "surface-1", 4.5],
  ["ink-primary", "accent-soft", 4.5],
  ["focus", "surface-0", 3],
  ["focus", "surface-1", 3],
  ["series-power", "surface-1", 3],
  ["series-hr", "surface-1", 3],
  ["series-cadence", "surface-1", 3],
  ["series-dfa", "surface-1", 3],
  ["series-torque", "surface-1", 3],
  ["target", "surface-1", 3],
  ["status-good", "surface-1", 4.5],
  ["status-warning", "surface-1", 4.5],
  ["status-critical", "surface-1", 4.5],
];

describe("palette", () => {
  for (const [mode, sel] of [
    ["dark", ':root[data-theme="dark"]'],
    ["light", ':root[data-theme="light"]'],
  ] as const) {
    test(`${mode}: every key pair meets its contrast minimum`, () => {
      const t = tokens(block(sel));
      for (const [fg, bg, min] of PAIRS) {
        expect(t[fg], `--${fg}`).toBeDefined();
        expect(t[bg], `--${bg}`).toBeDefined();
        expect(
          ratio(t[fg]!, t[bg]!),
          `--${fg} on --${bg}`,
        ).toBeGreaterThanOrEqual(min);
      }
    });
    test(`${mode}: defines every THEMES.md role`, () => {
      const t = tokens(block(sel));
      const roles = [
        ...[
          "surface-0",
          "surface-1",
          "surface-2",
          "border",
          "focus",
          "accent",
          "accent-ink",
          "accent-soft",
        ],
        ...[
          "ink-primary",
          "ink-secondary",
          "ink-muted",
          "target",
          "div-neg",
          "div-mid",
          "div-pos",
        ],
        ...[
          "status-good",
          "status-warning",
          "status-serious",
          "status-critical",
        ],
        ...[
          "series-power",
          "series-hr",
          "series-cadence",
          "series-dfa",
          "series-torque",
        ],
        ...[1, 2, 3, 4, 5, 6, 7].map((n) => `zone-${n}`),
        ...[1, 2, 3, 4, 5].map((n) => `fatigue-${n}`),
      ];
      for (const r of roles) expect(t[r], `--${r}`).toBeDefined();
    });
  }
});

describe("parseTheme", () => {
  test("accepts exactly the versioned shape", () => {
    expect(parseTheme(JSON.stringify({ v: 1, pref: "dark" }))).toBe("dark");
    for (const bad of [
      null,
      "",
      "x",
      '"dark"',
      "{}",
      '{"v":1}',
      '{"v":1,"pref":"Dark"}',
      "[]",
    ])
      expect(parseTheme(bad)).toBeNull();
  });
});
