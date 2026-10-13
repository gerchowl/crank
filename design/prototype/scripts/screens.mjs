// pnpm screens — renders every scene (src/scenes.ts) at each window size and theme into
// docs/design/screens/<scene>.<w>x<h>.<theme>.png (DESIGN §12). Scenes are seeded and frozen,
// so the output is deterministic. Fails on page errors, scene errors and horizontal overflow
// (ride scenes: any overflow — the ride screen must fit the window, #7), and on any [data-fit]
// element whose text is clipped (reserved rows never wrap or cut text, DESIGN §6.1). Runs the
// contrast check first (DESIGN §6.3), and checks every rendered chart mark's composited contrast.
import { execFileSync } from "node:child_process";
import { mkdirSync, readdirSync, rmSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";

const root = fileURLToPath(new URL("..", import.meta.url));
const out = fileURLToPath(new URL("../../../docs/design/screens/", import.meta.url));
const SIZES = [
  [1440, 900],
  [1024, 768],
];
const THEMES = ["dark", "light"];

execFileSync(process.execPath, [fileURLToPath(new URL("./contrast.mjs", import.meta.url))], { stdio: "inherit" });
mkdirSync(out, { recursive: true });
for (const f of readdirSync(out)) if (f.endsWith(".png")) rmSync(out + f);

const server = await createServer({ root, logLevel: "warn", server: { port: 0 } });
await server.listen();
const base = server.resolvedUrls?.local[0];
if (!base) throw new Error("vite did not report a local URL");
const browser = await chromium.launch();
const failures = [];

try {
  const lister = await browser.newPage();
  await lister.goto(`${base}?list-scenes=1`);
  const ids = JSON.parse((await lister.locator("body").getAttribute("data-scenes")) ?? "[]");
  await lister.close();
  if (!ids.length) throw new Error("no scenes listed");

  for (const [width, height] of SIZES) {
    const ctx = await browser.newContext({ viewport: { width, height }, deviceScaleFactor: 1, reducedMotion: "reduce" });
    for (const theme of THEMES) {
      for (const id of ids) {
        const name = `${id}.${width}x${height}.${theme}`;
        const page = await ctx.newPage();
        page.on("pageerror", (e) => failures.push(`${name}: ${e.message}`));
        page.on("console", (m) => m.type() === "error" && failures.push(`${name}: console: ${m.text()}`));
        await page.goto(`${base}?scene=${id}&theme=${theme}`);
        await page.waitForFunction(() => {
          const r = document.getElementById("root");
          return r && (r.dataset.sceneError || (r.childElementCount > 0 && !document.querySelector("[data-pending]")));
        });
        const sceneError = await page.locator("#root").getAttribute("data-scene-error");
        if (sceneError) failures.push(`${name}: scene error: ${sceneError}`);
        await page.evaluate(() => document.fonts.ready);
        const m = await page.evaluate(() => ({
          sw: document.documentElement.scrollWidth,
          sh: document.documentElement.scrollHeight,
          ride: !!document.querySelector(".ride"),
          clipped: [...document.querySelectorAll("[data-fit]")]
            .filter((el) => el.scrollWidth > el.clientWidth + 1 || el.scrollHeight > el.clientHeight + 1)
            .map((el) => `"${el.textContent?.trim().slice(0, 50)}" (${el.scrollWidth}×${el.scrollHeight} in ${el.clientWidth}×${el.clientHeight})`),
        }));
        for (const c of m.clipped) failures.push(`${name}: clipped text ${c}`);
        // rendered chart marks: effective stroke (opacity inherited from every ancestor) composited over
        // the real background must reach 3:1 (DESIGN §6.3, WCAG 1.4.11) — token checks alone miss this
        const weak = await page.evaluate(() => {
          const parse = (c) => {
            const m = /rgba?\(([^)]+)\)/.exec(c) ?? /color\(srgb ([^)]+)\)/.exec(c);
            if (!m) return null;
            const parts = m[1].replace("/", " ").split(/[\s,]+/).filter(Boolean).map(Number);
            const scale = c.startsWith("color(") ? 255 : 1;
            return { r: parts[0] * scale, g: parts[1] * scale, b: parts[2] * scale, a: parts[3] ?? 1 };
          };
          const lum = ({ r, g, b }) => [r, g, b].map((v) => v / 255).map((v) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4)).reduce((acc, v, i) => acc + v * [0.2126, 0.7152, 0.0722][i], 0);
          const bgOf = (el) => {
            for (let e = el; e; e = e.parentElement) {
              const c = parse(getComputedStyle(e).backgroundColor);
              if (c && c.a > 0.99) return c;
            }
            return parse(getComputedStyle(document.body).backgroundColor);
          };
          const out = [];
          for (const el of document.querySelectorAll("svg .line, svg .band-edge, svg .plan-line, svg .eased-line, svg .area-edge, svg .cap-line, svg .marker, svg rect.gap, svg .target-line")) {
            const cs = getComputedStyle(el);
            const st = parse(cs.stroke);
            if (!st || cs.stroke === "none") continue;
            let alpha = st.a * Number(cs.strokeOpacity || 1);
            for (let e = el; e && e.tagName !== "svg"; e = e.parentElement) alpha *= Number(getComputedStyle(e).opacity || 1);
            const bg = bgOf(el.closest("svg"));
            const mix = { r: st.r * alpha + bg.r * (1 - alpha), g: st.g * alpha + bg.g * (1 - alpha), b: st.b * alpha + bg.b * (1 - alpha) };
            const [x, y] = [lum(mix), lum(bg)].sort((p, q) => q - p);
            const ratio = (x + 0.05) / (y + 0.05);
            if (ratio < 3 && !el.closest("[data-decorative]")) out.push(`${el.getAttribute("class")} ${ratio.toFixed(2)}:1`);
          }
          return [...new Set(out)];
        });
        for (const w of weak) failures.push(`${name}: chart mark below 3:1 — ${w}`);
        if (m.sw > width) failures.push(`${name}: horizontal overflow (${m.sw} > ${width})`);
        if (m.ride && m.sh > height) failures.push(`${name}: ride screen overflows vertically (${m.sh} > ${height})`);
        await page.screenshot({ path: `${out}${name}.png`, fullPage: !m.ride, animations: "disabled" });
        console.log(`  ${name}.png`);
        await page.close();
      }
    }
    await ctx.close();
  }
} finally {
  await browser.close();
  await server.close();
}
if (failures.length) {
  console.error(`\n${failures.length} problem(s):\n${failures.join("\n")}`);
  process.exit(1);
}
