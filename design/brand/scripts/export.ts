// pnpm export — writes the chosen wordmark (A · Slice) as true outlines to assets/brand/ (BRAND.md §Files):
// the full form and the small form, each as a mono (currentColor) master plus light/dark two-colour variants.
// Plain filled paths only: no clipPath, no mask, no ids, so the files can be inlined any number of times.
import { mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { RECOMMENDED, small, svg, type Paint } from "../src/wordmark.ts";

const out = fileURLToPath(new URL("../../../assets/brand/", import.meta.url));
// Hex mirrors src/style.css; the assets must not depend on CSS.
const PAINTS: Record<string, Paint> = {
  "": { ink: "currentColor", accent: "currentColor" },
  "-dark": { ink: "#ece6dc", accent: "#c08a4a" }, // on dark surfaces (graphite #1a1d22)
  "-light": { ink: "#1a1d22", accent: "#94642d" }, // on light surfaces (paper #f4f1ea)
};
const a11y = ' role="img" aria-label="crank"';

mkdirSync(out, { recursive: true });
for (const [suffix, paint] of Object.entries(PAINTS)) {
  const files = {
    [`wordmark${suffix}.svg`]: svg(RECOMMENDED, paint, "", a11y),
    [`wordmark-small${suffix}.svg`]: small(paint).replace("<svg ", `<svg${a11y} `),
  };
  for (const [file, markup] of Object.entries(files)) {
    // Motion/layout hooks (class, style, --i) belong to the start screen only; the static asset drops them.
    const clean = markup.replace(/ (class|style)="[^"]*"/g, "");
    if (/clipPath|mask|\bid=/.test(clean)) throw new Error(`${file}: not plain outlines`);
    writeFileSync(out + file, `${clean}\n`);
    console.log(`  assets/brand/${file}`);
  }
}
