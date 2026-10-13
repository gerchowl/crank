// pnpm screens — renders into docs/design/brand/ (BRAND.md §Files): every concept's start screen at each window
// size and theme, the overview sheet, frames of each launch motion, the slow-boot and hand-off states of A, and
// the size-proof sheets of the exported assets. Static shots use reduced motion, which is the final frame by
// construction; timers run on Playwright's fake clock, so boot timing is deterministic. Fails on page errors,
// overflow, a wordmark leaving the window, motion over 1.2 s, wrong boot-status or hand-off behaviour, and any
// size-sheet item not rendered at its stated width.
import { mkdirSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium, type Page } from "playwright";
import { CONCEPTS } from "../src/wordmark.ts";
import { SLICE } from "../src/geometry.ts";
import { SLICE_ART } from "../src/slice.ts";

const root = fileURLToPath(new URL("..", import.meta.url));
const out = fileURLToPath(new URL("../../../docs/design/brand/", import.meta.url));
const SIZES = [
  [1440, 900],
  [1024, 768],
] as const;
const THEMES = ["dark", "light"] as const;
const FRAMES = [0, 150, 300, 500, 750, 1200]; // ms; 1200 = the motion budget, everything at rest
const ids = Object.keys(CONCEPTS);

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
  await page.clock.install();
  await page.goto(`${base}?${query}`);
  await page.waitForSelector("svg");
  const m = await page.evaluate(() => {
    const r = document.querySelector(".start .wm svg")?.getBoundingClientRect();
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
          if (typeof end === "number" && true) longest = Math.max(longest, end);
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
  await bootAndHandOff();
  for (const theme of THEMES) await sizeSheet(theme);
} finally {
  await browser.close();
  await server.close();
}
/** A's boot timing and hand-off, with and without reduced motion (BRAND.md §Start screen). */
async function bootAndHandOff() {
  for (const reducedMotion of ["reduce", "no-preference"] as const) {
    const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1, reducedMotion });
    for (const theme of THEMES) {
      const tag = `${reducedMotion}.${theme}`;
      let page = await ctx.newPage();
      await open(page, `slow-boot.${tag}`, `concept=slice&theme=${theme}`);
      await page.clock.runFor(1150);
      if (await page.locator(".status").count()) failures.push(`slow-boot.${tag}: status shown before 1.2 s`);
      await page.clock.runFor(100);
      if (!(await page.locator('.status[role="status"]').count())) failures.push(`slow-boot.${tag}: no status after 1.2 s`);
      if (reducedMotion === "reduce") await page.screenshot({ path: `${out}slice.slow-boot.1440x900.${theme}.png` });
      await page.close();

      page = await ctx.newPage();
      await open(page, `handoff.${tag}`, `concept=slice&theme=${theme}&ready=600`);
      await page.clock.runFor(600);
      const s = await page.evaluate(() => ({
        start: !!document.querySelector(".start"),
        top: document.elementFromPoint(innerWidth / 2, innerHeight / 2)?.closest(".dest") !== null,
        focused: document.activeElement?.closest(".dest") !== null,
        status: !!document.querySelector(".status"),
      }));
      if (!s.top || !s.focused) failures.push(`handoff.${tag}: destination not interactive as the hand-off begins`);
      if (s.status) failures.push(`handoff.${tag}: status shown on a 600 ms boot`);
      if (reducedMotion === "reduce" && s.start) failures.push(`handoff.${tag}: start screen not removed at once under reduced motion`);
      if (reducedMotion === "no-preference") {
        if (!s.start) failures.push(`handoff.${tag}: no cross-fade`);
        await page.evaluate(() => {
          // CSS launch motion runs on the document timeline, not the fake clock: put it where it was frozen
          // (ready at 600 ms), then show the 200 ms cross-fade half-way.
          const start = document.querySelector(".start")!;
          const fade = start.getAnimations();
          for (const a of start.getAnimations({ subtree: true })) {
            a.pause();
            a.currentTime = fade.includes(a) ? 100 : 600;
          }
        });
        await page.screenshot({ path: `${out}slice.handoff-100ms.1440x900.${theme}.png` });
      }
      await page.close();
    }
    await ctx.close();
  }
  console.log("  slice.slow-boot.*.png, slice.handoff-100ms.*.png");
}

/** Size proof: every item at its stated size with shrinking disabled; labels are measured, not assumed. */
async function sizeSheet(theme: (typeof THEMES)[number]) {
  const assets = fileURLToPath(new URL("../../../assets/", import.meta.url));
  const file = (f: string) => readFileSync(assets + f, "utf8");
  const [bg, ink, bar] = theme === "dark" ? ["#1a1d22", "#ece6dc", "#24282f"] : ["#f4f1ea", "#1a1d22", "#e6e1d6"];
  const full = file(`brand/wordmark-${theme}.svg`);
  const smallForm = file(`brand/wordmark-small-${theme}.svg`);
  const icon = file("icon/logo.svg");
  // The size goes on the <svg> itself (no container sizing, no flex shrinking), so the label measures the art.
  const item = (svg: string, size: string, kind: string) =>
    `<div class="it" data-kind="${kind}">${svg.replace(/<svg /, `<svg style="${size}" `)}<span></span></div>`;
  const row = (title: string, items: string) => `<h2>${title}</h2><div class="row">${items}</div>`;
  const titleBar = (svg: string, size: string, kind: string, note: string) =>
    `<div class="bar">${item(icon, "height:16px", "mark")}${item(svg, size, kind)}<em>${note}</em></div>`;
  const page = await browser.newPage({ viewport: { width: 1200, height: 900 }, deviceScaleFactor: 2 });
  page.on("pageerror", (e) => failures.push(`sizes.${theme}: ${e.message}`));
  await page.setContent(`<style>
    body { margin: 0; padding: 32px 48px; background: ${bg}; color: ${ink}; font: 13px system-ui, sans-serif; }
    h2 { font-size: 13px; font-weight: 600; margin: 28px 0 12px; opacity: .8 }
    .row { display: flex; gap: 40px; align-items: flex-end; flex-wrap: nowrap; }
    .it { flex: none; display: grid; gap: 8px; justify-items: start; } .it svg { display: block; height: auto; }
    .it svg[style*="height"] { width: auto; } .it span { font-variant-numeric: tabular-nums; opacity: .7; white-space: nowrap }
    .bar { display: flex; align-items: center; gap: 8px; height: 24px; width: 560px; padding: 0 10px; background: ${bar}; border-radius: 6px; margin-bottom: 40px }
    .bar .it { grid-auto-flow: column; align-items: center; } .bar .it span { font-size: 11px } .bar em { margin-left: auto; opacity: .7 }
  </style>
  ${row("Full form — wordmark-" + theme + ".svg (use at ≥ 160 px wide)", [480, 240, 160].map((w) => item(full, `width:${w}px`, "full")).join(""))}
  ${row("Small form — wordmark-small-" + theme + ".svg (below 160 px; cap height ≥ 9 px)", [150, 100, 64, 44].map((w) => item(smallForm, `width:${w}px`, "small")).join(""))}
  ${row("App mark — below a 9 px cap height", [32, 24, 16].map((w) => item(icon, `width:${w}px;height:${w}px`, "mark")).join(""))}
  ${row("Title bar, 24 px high", "")}
  ${titleBar(smallForm, "height:12px", "small", "do: mark + small form (cap 10 px)")}
  ${titleBar(full, "height:24px", "full", "don't: full form at 24 px high — cuts close up")}`);
  const cutPerPx = SLICE.g / SLICE_ART.box[2];
  const bad = await page.evaluate((cut) => {
    const wrong: string[] = [];
    for (const it of document.querySelectorAll<HTMLElement>(".it")) {
      const svg = it.querySelector("svg")!;
      const r = svg.getBoundingClientRect();
      const want = parseFloat(svg.style.width) || parseFloat(svg.style.height);
      const got = svg.style.width ? r.width : r.height;
      if (Math.abs(got - want) > 0.5) wrong.push(`${it.dataset.kind} ${got.toFixed(1)} ≠ ${want}`);
      it.querySelector("span")!.textContent =
        `${r.width.toFixed(0)} × ${r.height.toFixed(0)} px` + (it.dataset.kind === "full" ? ` · cut ${(cut * r.width).toFixed(2)} px` : "");
    }
    return wrong;
  }, cutPerPx);
  for (const b of bad) failures.push(`sizes.${theme}: ${b}`);
  await page.screenshot({ path: `${out}sizes.${theme}.png`, fullPage: true });
  console.log(`  sizes.${theme}.png`);
  await page.close();
}

if (failures.length) {
  console.error(`\n${failures.length} problem(s):\n${failures.join("\n")}`);
  process.exit(1);
}
