// pnpm screens — renders every concept's start screen at each window size and theme, the overview sheet,
// frames of each launch motion, and the exported assets at small sizes into docs/design/brand/ (BRAND.md §Screens). Static shots use
// reduced motion, which is the final frame by construction. Motion frames pause every animation at t.
// Fails on page errors, horizontal overflow, and a wordmark that leaves the window.
import { mkdirSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium, type Page } from "playwright";
import { CONCEPTS } from "../src/wordmark.ts";

const root = fileURLToPath(new URL("..", import.meta.url));
const out = fileURLToPath(new URL("../../../docs/design/brand/", import.meta.url));
const SIZES = [
  [1440, 900],
  [1024, 768],
] as const;
const THEMES = ["dark", "light"] as const;
const FRAMES = [0, 150, 300, 500, 750, 1200]; // ms; 1200 = the motion budget, everything at rest
const ids = Object.keys(CONCEPTS);
const ASSET_WIDTHS = [480, 240, 140, 96, 64];

mkdirSync(out, { recursive: true });
for (const f of readdirSync(out)) if (f.endsWith(".png")) rmSync(out + f);

const server = await createServer({ root, logLevel: "warn", server: { port: 0 } });
await server.listen();
const base = server.resolvedUrls?.local[0];
if (!base) throw new Error("vite did not report a local URL");
const browser = await chromium.launch();
const failures: string[] = [];

async function open(page: Page, name: string, query: string) {
  page.on("pageerror", (e) => failures.push(`${name}: ${e.message}`));
  page.on("console", (m) => m.type() === "error" && failures.push(`${name}: console: ${m.text()}`));
  await page.goto(`${base}?${query}`);
  await page.waitForSelector("svg");
  const m = await page.evaluate(() => {
    const r = document.querySelector(".start .wm")?.getBoundingClientRect();
    return { sw: document.documentElement.scrollWidth, wm: r && { l: r.left, r: r.right, t: r.top, b: r.bottom } };
  });
  const { width: w, height: h } = page.viewportSize()!;
  if (m.sw > w) failures.push(`${name}: horizontal overflow (${m.sw} > ${w})`);
  if (m.wm && (m.wm.l < 0 || m.wm.r > w || m.wm.t < 0 || m.wm.b > h)) failures.push(`${name}: wordmark leaves the window`);
}

try {
  for (const [width, height] of SIZES) {
    const ctx = await browser.newContext({ viewport: { width, height }, deviceScaleFactor: 1, reducedMotion: "reduce" });
    for (const theme of THEMES) {
      for (const id of [...ids, "overview"]) {
        const name = `${id}.${width}x${height}.${theme}`;
        const page = await ctx.newPage();
        await open(page, name, id === "overview" ? `theme=${theme}` : `concept=${id}&theme=${theme}`);
        await page.screenshot({ path: `${out}${name}.png` });
        console.log(`  ${name}.png`);
        await page.close();
      }
    }
    await ctx.close();
  }
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1, reducedMotion: "no-preference" });
  for (const id of ids) {
    for (const t of FRAMES) {
      const name = `motion.${id}.${String(t).padStart(4, "0")}ms`;
      const page = await ctx.newPage();
      await open(page, name, `concept=${id}&theme=dark`);
      const over = await page.evaluate((at) => {
        let longest = 0;
        for (const a of document.getAnimations()) {
          const end = a.effect?.getComputedTiming().endTime;
          if (typeof end === "number" && (a as CSSAnimation).animationName !== "status") longest = Math.max(longest, end);
          a.pause();
          a.currentTime = at;
        }
        return longest;
      }, t);
      if (over > 1200) failures.push(`${name}: launch motion lasts ${over} ms (> 1200)`);
      await page.screenshot({ path: `${out}${name}.png` });
      console.log(`  ${name}.png`);
      await page.close();
    }
  }
  await ctx.close();
  // The exported assets (assets/brand/, from scripts/export.ts) at decreasing widths: the min-size check.
  const assets = fileURLToPath(new URL("../../../assets/brand/", import.meta.url));
  const page = await browser.newPage({ viewport: { width: 1200, height: 640 }, deviceScaleFactor: 1 });
  const row = (file: string, bg: string, color: string) =>
    `<div style="background:${bg};color:${color};display:flex;gap:48px;align-items:center;padding:40px 48px">` +
    ASSET_WIDTHS.map((w) => `<div style="width:${w}px">${readFileSync(assets + file, "utf8")}</div>`).join("") +
    `</div>`;
  await page.setContent(
    `<body style="margin:0">${row("wordmark-dark.svg", "#1a1d22", "")}${row("wordmark-light.svg", "#f4f1ea", "")}` +
      `${row("wordmark.svg", "#ffffff", "#1a1d22")}${row("wordmark.svg", "#1a1d22", "#ece6dc")}</body>`,
  );
  await page.screenshot({ path: `${out}assets.png`, fullPage: true });
  console.log("  assets.png");
  await page.close();
} finally {
  await browser.close();
  await server.close();
}
if (failures.length) {
  console.error(`\n${failures.length} problem(s):\n${failures.join("\n")}`);
  process.exit(1);
}
