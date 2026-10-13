// Contrast check for the key token pairs (DESIGN §6.3): text ≥ 4.5:1, large text / chart marks /
// UI boundaries ≥ 3:1, in both modes. Reads src/theme.css; exits non-zero on any failure.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const css = readFileSync(fileURLToPath(new URL("../src/theme.css", import.meta.url)), "utf8");
const block = (sel) => {
  const i = css.indexOf(sel);
  if (i < 0) throw new Error(`theme.css: ${sel} not found`);
  return css.slice(i, css.indexOf("}", i));
};
const tokens = (b) => Object.fromEntries([...b.matchAll(/--([\w-]+):\s*(#[0-9a-f]{6})/gi)].map((m) => [m[1], m[2]]));
const lum = (hex) => {
  const c = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255).map((v) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
  return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
};
const ratio = (a, b) => {
  const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p);
  return (x + 0.05) / (y + 0.05);
};
// [foreground, background, minimum, what]
const PAIRS = [
  ["ink-primary", "surface-0", 4.5, "text"],
  ["ink-secondary", "surface-1", 4.5, "text"],
  ["ink-muted", "surface-1", 4.5, "text"],
  ["ink-muted", "surface-2", 4.5, "text"],
  ["accent-ink", "accent", 4.5, "button text"],
  ["accent-text", "surface-1", 4.5, "accent text"],
  ["series-power", "surface-1", 3, "chart mark"],
  ["series-hr", "surface-1", 3, "chart mark"],
  ["series-cadence", "surface-1", 3, "chart mark"],
  ["series-muted", "surface-1", 3, "chart mark"],
  ["target", "surface-1", 3, "chart mark"],
  ["div-neg", "surface-0", 4.5, "Δ text"],
  ["div-pos", "surface-0", 4.5, "Δ text"],
  ["status-good", "surface-1", 4.5, "status text"],
  ["status-info", "surface-1", 4.5, "status text"],
  ["status-warning", "surface-1", 4.5, "status text"],
  ["status-serious", "surface-1", 4.5, "status text"],
  ["accent", "surface-1", 3, "wish matrix: planned cell (UI boundary)"],
  ["accent-text", "surface-2", 3, "wish matrix: wished-cell outline (UI boundary)"],
  ["status-warning", "surface-2", 3, "wish matrix: cap edge (UI boundary)"],
  ["focus", "surface-1", 3, "wish mode cursor (UI boundary)"],
  ["warning-ink", "status-warning", 4.5, "over-cap status text on its warning fill"],
  ["border", "surface-1", 1.3, "hairline (decorative)"],
];
let failed = 0;
for (const [mode, sel] of [
  ["dark", ':root[data-theme="dark"]'],
  ["light", ':root[data-theme="light"]'],
]) {
  const t = tokens(block(sel));
  for (const [fg, bg, min, what] of PAIRS) {
    if (!t[fg] || !t[bg]) throw new Error(`${mode}: missing --${fg} or --${bg}`);
    const r = ratio(t[fg], t[bg]);
    if (r < min) {
      failed++;
      console.error(`  ✗ ${mode} --${fg} on --${bg}: ${r.toFixed(2)}:1 < ${min}:1 (${what})`);
    }
  }
}
if (failed) {
  console.error(`contrast: ${failed} pair(s) below minimum`);
  process.exit(1);
}
console.log(`contrast: ${PAIRS.length * 2} pairs pass`);
