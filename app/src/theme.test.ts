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
// Colour tokens of a block as hex; an alias (`--ink-primary: var(--ink)`) resolves to its target in the same block.
const tokens = (b: string) => {
  const raw = Object.fromEntries(
    [...b.matchAll(/--([\w-]+):\s*(#[0-9a-f]{6}|var\(--[\w-]+\))/gi)].map(
      (m) => [m[1]!, m[2]!],
    ),
  );
  const resolve = (v: string): string => {
    const a = /^var\(--([\w-]+)\)$/.exec(v);
    if (!a) return v;
    const t = raw[a[1]!];
    if (t === undefined) throw new Error(`theme.css: --${a[1]} undefined`);
    return resolve(t);
  };
  return Object.fromEntries(
    Object.entries(raw).map(([k, v]) => [k, resolve(v)]),
  );
};
// Scale tokens (px lengths) of the mode-independent `:root` block, by name prefix.
const px = (prefix: string) =>
  Object.fromEntries(
    [...block(":root {").matchAll(/--([\w-]+):\s*(\d+)px/g)]
      .filter((m) => m[1]!.startsWith(prefix))
      .map((m) => [m[1]!, Number(m[2])]),
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
  ["warning-ink", "status-warning", 4.5],
  // DESIGN §18.1 graphite palette (§6.3 floors; rule 7). Text 4.5:1, marks and control edges 3:1 (WCAG 2.2 1.4.11).
  ["ink", "bg", 4.5],
  ["ink", "panel", 4.5],
  ["ink-2", "panel", 4.5],
  ["ink-2", "panel-2", 4.5],
  ["ink-3", "bg", 4.5],
  ["ink-3", "panel", 4.5],
  ["ink-3", "panel-2", 4.5],
  ["power", "panel", 3],
  ["power", "bg", 3],
  ["hr", "panel", 3],
  ["hr", "bg", 3],
  ["cad", "panel", 3],
  ["cad", "bg", 3],
  ["target", "panel", 3],
  ["good", "panel", 4.5],
  ["warn", "panel", 4.5],
  ["edge", "panel", 3],
  ["edge", "bg", 3],
  ["focus", "panel-2", 3],
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

// DESIGN §18.1 scales: the tokens are the whole vocabulary of rule 2, so their values are pinned here.
describe("scales", () => {
  test("every --space-* is a multiple of 4 and the set is the §18.1 scale", () => {
    const space = px("space-");
    for (const [k, v] of Object.entries(space)) expect(v % 4, `--${k}`).toBe(0);
    expect(Object.values(space)).toEqual([4, 8, 12, 16, 24, 32]);
  });
  test("--radius-* are 4, 6 and 10", () => {
    expect(px("radius-")).toEqual({
      "radius-s": 4,
      "radius-m": 6,
      "radius-l": 10,
    });
  });
  test("at most 6 --text-* tokens, and the ride tiers are 112, 64 and 40 (rule 16)", () => {
    const text = px("text-");
    expect(Object.keys(text).length).toBeLessThanOrEqual(6);
    expect(Object.values(text)).toEqual([12, 13, 15, 20, 32]);
    const tier = px("tier-");
    expect(tier["tier-t1"]).toBe(112);
    expect(tier["tier-t2"]).toBe(64);
    expect(tier["tier-t3plus"]).toBe(40);
  });
  test("--control-h is 32px, and 40 and 48 are the only other heights", () => {
    const c = px("control-h");
    expect(c["control-h"]).toBe(32);
    expect(Object.values(c).sort((a, b) => a - b)).toEqual([32, 40, 48]);
  });
  test("every size token, in every block, is a px length", () => {
    // `px()` reads only the shared `:root` block; this reads every declaration, so a rem or a theme-block size gets caught.
    const sizes = [
      ...css.matchAll(
        /--((?:space|radius|text|tier|control-h)(?:-[\w-]+)?):\s*([^;]+);/g,
      ),
    ];
    expect(sizes.length).toBeGreaterThan(0);
    for (const [, name, value] of sizes)
      expect(value, `--${name}`).toMatch(/^\d+px$/);
  });
  test("the fonts are local and swap, with no remote source", () => {
    expect(css).toMatch(/--font-ui:\s*Inter,/);
    expect(css).toMatch(/--font-mono:\s*"JetBrains Mono",/);
    const faces = css.match(/@font-face\s*{[^}]*}/g) ?? [];
    expect(faces.length).toBeGreaterThan(0);
    for (const f of faces) {
      expect(f).toContain("font-display: swap");
      expect(f).toMatch(/url\("\.\/assets\/fonts\/[\w-]+\.woff2"\)/);
      expect(f).not.toMatch(/https?:/);
    }
  });
  test("numerics use tabular figures and a slashed zero", () => {
    expect(css).toMatch(/font-variant-numeric:\s*tabular-nums slashed-zero/);
  });
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
