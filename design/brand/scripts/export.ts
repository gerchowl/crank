// pnpm export — writes the recommended wordmark (src/wordmark.ts RECOMMENDED) as outlined SVG to
// assets/brand/ (BRAND.md §Files): one mono (currentColor) master plus the two-colour light/dark variants.
import { mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { RECOMMENDED, svg, type Paint } from "../src/wordmark.ts";

const out = fileURLToPath(new URL("../../../assets/brand/", import.meta.url));
// Hex mirrors src/style.css; the assets must not depend on CSS.
const VARIANTS: Record<string, Paint> = {
  "wordmark.svg": { ink: "currentColor", accent: "currentColor" },
  "wordmark-dark.svg": { ink: "#ece6dc", accent: "#c08a4a" }, // on dark surfaces (graphite #1a1d22)
  "wordmark-light.svg": { ink: "#1a1d22", accent: "#94642d" }, // on light surfaces (paper #f4f1ea)
};

mkdirSync(out, { recursive: true });
for (const [file, paint] of Object.entries(VARIANTS)) {
  const uid = file.replace(".svg", "");
  // Motion hooks (class, --i) are for the start screen only; the static asset drops them.
  const markup = svg(RECOMMENDED, paint, uid, ' role="img" aria-label="crank"').replace(/ (class|style)="[^"]*"/g, "");
  writeFileSync(out + file, `${markup}\n`);
  console.log(`  assets/brand/${file}`);
}
