// pnpm screens — renders every scene (src/scenes.ts) at each window size and theme into
// docs/design/screens/<scene>.<w>x<h>.<theme>.png (DESIGN §12). Scenes are seeded and frozen,
// so the output is deterministic. Fails on page errors, scene errors and horizontal overflow
// (ride scenes: any overflow — the ride screen must fit the window, #7), and on any [data-fit]
// element whose text is clipped (reserved rows never wrap or cut text, DESIGN §6.1). Runs the
// contrast check first (DESIGN §6.3).
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
