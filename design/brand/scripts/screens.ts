// pnpm screens — renders into docs/design/brand/ (BRAND.md §Files): every concept's start screen at each window
// size and theme, the overview sheet, frames of each launch motion, the slow-boot and hand-off states of A, and
// the size-proof sheets of the exported assets. Static shots use reduced motion, which is the final frame by
// construction; timers run on Playwright's fake clock, so boot timing is deterministic. Fails on page errors,
// overflow, a wordmark leaving the window, motion over 1.2 s, wrong boot-status or hand-off behaviour, and any
// size-sheet item not rendered at its stated width. F · Spin-up (#83) adds its storyboard, small-size strip,
// boot behaviours and the A-vs-F sheet, and fails unless its final frame is pixel-identical to A at rest.
import { mkdirSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium, type Page } from "playwright";
import { CONCEPTS } from "../src/wordmark.ts";
import { SLICE } from "../src/geometry.ts";
import { SLICE_ART } from "../src/slice.ts";
import { LETTER_START, SPIN } from "../src/spin.ts";

const root = fileURLToPath(new URL("..", import.meta.url));
const out = fileURLToPath(new URL("../../../docs/design/brand/", import.meta.url));
const SIZES = [
  [1440, 900],
  [1024, 768],
] as const;
const THEMES = ["dark", "light"] as const;
const FRAMES = [0, 150, 300, 500, 750, 1200]; // ms; 1200 = the motion budget, everything at rest
const ids = Object.keys(CONCEPTS);
const STORY = Array.from({ length: 13 }, (_, i) => i * 100); // F's storyboard: every 100 ms, 0 … 1200
const spinOut = `${out}spin/`;

for (const dir of [out, spinOut]) {
  mkdirSync(dir, { recursive: true });
  for (const f of readdirSync(dir)) if (f.endsWith(".png")) rmSync(dir + f);
}

const server = await createServer({ root, logLevel: "warn", server: { port: 0 } });
await server.listen();
const base = server.resolvedUrls?.local[0];
if (!base) throw new Error("vite did not report a local URL");
const browser = await chromium.launch();
const failures: string[] = [];
/** Static start screens, kept for the A-vs-F exactness check: `${id}.${w}x${h}.${theme}`. */
const shots = new Map<string, Buffer>();
const pad = (t: number) => String(t).padStart(4, "0");
/** F's storyboard frames by `${w}x${h}.${theme}`, reused by the A-vs-F sheet. */
const spinFrames = new Map<string, { label: string; png: Buffer }[]>();

// F's budget, from its own timing table: everything at rest by SPIN.end ≤ 1100 ms (headroom under the 1.2 s budget).
const spinEnds = [...LETTER_START.map((s) => s + SPIN.letter), SPIN.slice[0] + SPIN.slice[1], SPIN.morph[1]];
if (SPIN.end > 1100 || Math.max(...spinEnds) > SPIN.end) failures.push(`spin: motion ends at ${Math.max(...spinEnds)} ms (end ${SPIN.end}, target 1100)`);
console.log(`  spin: R A N K start at ${LETTER_START.join(" / ")} ms; at rest by ${Math.max(...spinEnds)} ms`);

/** `paused`: the fake clock moves only by `runFor` (installed clocks otherwise also flow in real time). */
async function open(page: Page, name: string, query: string, paused = false) {
  page.on("pageerror", (e) => failures.push(`${name}: ${e.message}`));
  page.on("console", (m) => m.type() === "error" && failures.push(`${name}: console: ${m.text()}`));
  await page.clock.install({ time: 0 });
  if (paused) await page.clock.pauseAt(1000);
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
        shots.set(name, await page.screenshot({ path: `${out}${name}.png` }));
        console.log(`  ${name}.png`);
        await page.close();
      }
    }
    await ctx.close();
  }
  await spinStoryboards();
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1, reducedMotion: "no-preference" });
  for (const id of ids.filter((i) => i !== "spin")) {
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
  await spinBoot();
  await spinEarly();
  for (const theme of THEMES) await spinSmall(theme);
  for (const theme of THEMES) await compareSheet(theme);
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

/** F every 100 ms at both window sizes and themes (?t= renders the frozen frame), its storyboard sheets, and
 *  the landing check: the driven final frame and F under reduced motion must equal A at rest, pixel for pixel. */
async function spinStoryboards() {
  for (const [width, height] of SIZES) {
    const ctx = await browser.newContext({ viewport: { width, height }, deviceScaleFactor: 1, reducedMotion: "no-preference" });
    for (const theme of THEMES) {
      const size = `${width}x${height}.${theme}`;
      const frames: { label: string; png: Buffer }[] = [];
      for (const t of STORY) {
        const name = `spin.${size}.${pad(t)}ms`;
        const page = await ctx.newPage();
        await open(page, name, `concept=spin&theme=${theme}&t=${t}`);
        frames.push({ label: `${t} ms`, png: await page.screenshot({ path: `${spinOut}${name}.png` }) });
        await page.close();
      }
      spinFrames.set(size, frames);
      const a = shots.get(`slice.${size}`)!;
      for (const [what, png] of [
        ["final frame (t = 1200)", frames.at(-1)!.png],
        ["reduced-motion frame", shots.get(`spin.${size}`)!],
      ] as const) {
        const d = await pixelDiff(a, png);
        if (d !== 0) failures.push(`spin.${size}: ${what} differs from A at rest in ${d} px`);
      }
      await sheet(`${out}spin.storyboard.${size}.png`, `F · Spin-up — ${width}×${height}, ${theme}, every 100 ms`, theme, 4, 320, frames);
      console.log(`  spin/spin.${size}.*ms.png (${frames.length}), spin.storyboard.${size}.png`);
    }
    await ctx.close();
  }
}

/** F's boot behaviours on the fake clock (BRAND.md §F · Spin-up → Boot): (a) on ready the motion runs on at 1:1
 *  under the 200 ms hand-off fade (early readiness: `spinEarly`); (b) a slow boot plays through and rests,
 *  status at 1.2 s; (b′, ?hold) the closed ring holds until ready. Destination live + focused and the
 *  start screen inert as every hand-off begins; under reduced motion: no motion, status at 1.2 s, removed at once. */
async function spinBoot() {
  const state = (page: Page) =>
    page.evaluate(() => {
      const s = document.querySelector<HTMLElement>(".start");
      return {
        t: s?.dataset.t === undefined ? null : Number(s.dataset.t),
        opacity: s ? Number(getComputedStyle(s).opacity) : null,
        inert: !!s?.inert,
        status: !!document.querySelector(".status"),
        live:
          document.activeElement?.closest(".dest") !== null &&
          document.elementFromPoint(innerWidth / 2, innerHeight / 2)?.closest(".dest") !== null,
      };
    });
  for (const reducedMotion of ["no-preference", "reduce"] as const) {
    const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1, reducedMotion });
    const page = async (name: string, query: string) => {
      const p = await ctx.newPage();
      await open(p, name, `concept=spin&${query}`, true);
      return p;
    };
    for (const theme of THEMES) {
      const tag = `${reducedMotion}.${theme}`;
      const fail = (m: string) => failures.push(`spin-boot.${tag}: ${m}`);
      const handOffBegins = async (p: Page, what: string, slow = false) => {
        const s = await state(p);
        if (!s.live || (s.opacity !== null && !s.inert)) fail(`${what}: destination not live + focused, or start screen not inert, as the hand-off begins`);
        if (s.status !== slow) fail(`${what}: status ${s.status ? "shown" : "missing"} on a boot ${slow ? "over" : "under"} 1.2 s`);
        return s;
      };

      // (b) slow boot: plays through, rests settled; the status line comes from the boot timer at 1.2 s.
      let p = await page(`spin-slow.${tag}`, `theme=${theme}`);
      await p.clock.runFor(SPIN.end - 60);
      let s = await state(p);
      if (reducedMotion === "reduce" ? s.t !== null : s.t === null || s.t < SPIN.end - 80 || s.t >= SPIN.end) fail(`painted t = ${s.t} at ${SPIN.end - 60} ms`);
      await p.clock.runFor(1150 - (SPIN.end - 60));
      s = await state(p);
      if (s.status) fail("status before 1.2 s");
      if (reducedMotion === "no-preference" && s.t !== SPIN.end) fail(`not settled at 1150 ms (t = ${s.t})`);
      await p.clock.runFor(100);
      s = await state(p);
      if (!s.status) fail("no status after 1.2 s");
      if (reducedMotion === "no-preference" && s.t !== SPIN.end) fail(`not at rest at 1250 ms (t = ${s.t})`);
      await p.close();

      // (a) ready at 600 ms: runs on at 1:1 under the fade (removed at once under reduced motion).
      p = await page(`spin-handoff.${tag}`, `theme=${theme}&ready=600`);
      await p.clock.runFor(600);
      s = await handOffBegins(p, "ready at 600 ms");
      if (reducedMotion === "reduce") {
        if (s.opacity !== null) fail("start screen not removed at once under reduced motion");
        await p.close();
        continue;
      }
      await p.clock.runFor(100);
      s = await state(p);
      if (s.t === null || Math.abs(s.t - 700) > 17 || s.opacity === null || s.opacity < 0.4 || s.opacity > 0.6)
        fail(`100 ms into the hand-off: t = ${s.t}, opacity = ${s.opacity} (expected ≈ 700, half faded)`);
      await p.screenshot({ path: `${out}spin.handoff-100ms.1440x900.${theme}.png` });
      await p.clock.runFor(120);
      if (await p.locator(".start").count()) fail("start screen still there 220 ms after ready");
      await p.close();

      // (b′) ?hold: the closed ring holds (t stays at SPIN.up) past 1.2 s; on ready it runs on under the fade.
      p = await page(`spin-hold.${tag}`, `theme=${theme}&hold&ready=2000`);
      await p.clock.runFor(1900);
      s = await state(p);
      if (s.t !== SPIN.up || !s.status) fail(`hold: t = ${s.t}, status ${s.status} at 1.9 s (expected ${SPIN.up}, shown)`);
      // The status fade-in is CSS (document timeline, not the fake clock): show it as it is 700 ms after insertion.
      await p.evaluate(() => document.querySelector(".status")?.getAnimations().forEach((a) => a.finish()));
      await p.screenshot({ path: `${out}spin.hold.1440x900.${theme}.png` });
      await p.clock.runFor(100);
      await handOffBegins(p, "hold, ready at 2 s", true);
      await p.clock.runFor(220);
      if (await p.locator(".start").count()) fail("hold: start screen still there 220 ms after ready");
      await p.close();
    }
    await ctx.close();
  }
  console.log("  spin.handoff-100ms.*.png, spin.hold.*.png (boot behaviours checked)");
}

/** Early readiness (BRAND.md §F · Spin-up → Boot): ready at 149 / 151 / 300 / 450 ms — either side of round 1's
 *  150 ms cut, mid spin-up, the ring closing — at both sizes and themes, real-time frames 0 / 50 / 100 / 150 ms
 *  into the hand-off. Fails unless the destination is live + focused at once, t runs on at 1:1 (no snap, no
 *  fast-forward), the fade is linear, the screen is gone after 200 ms, and 149 vs 151 differ only by 2 ms of motion. */
async function spinEarly() {
  const READY = [149, 151, 300, 450];
  const AFTER = [0, 50, 100, 150];
  for (const [width, height] of SIZES) {
    const ctx = await browser.newContext({ viewport: { width, height }, deviceScaleFactor: 1, reducedMotion: "no-preference" });
    for (const theme of THEMES) {
      const size = `${width}x${height}.${theme}`;
      const fail = (m: string) => failures.push(`spin-early.${size}: ${m}`);
      const cells: { label: string; png: Buffer }[] = [];
      const mid = new Map<number, Buffer>();
      for (const r of READY) {
        const p = await ctx.newPage();
        await open(p, `spin-early.${size}.${r}`, `concept=spin&theme=${theme}&ready=${r}`, true);
        await p.clock.runFor(r);
        let elapsed = 0;
        for (const a of AFTER) {
          await p.clock.runFor(a - elapsed);
          elapsed = a;
          const s = await p.evaluate(() => {
            const el = document.querySelector<HTMLElement>(".start");
            return {
              t: Number(el?.dataset.t),
              opacity: el ? Number(getComputedStyle(el).opacity) : null,
              inert: !!el?.inert,
              live: document.activeElement?.closest(".dest") !== null,
            };
          });
          if (!s.live || !s.inert) fail(`ready ${r} ms, +${a} ms: destination not live + focused or start screen not inert`);
          // rAF on the fake clock paints every ~16 ms: the painted t trails the clock by less than a frame.
          if (a > 0 && (Math.abs(s.t - (r + a)) > 17 || s.opacity === null || Math.abs(s.opacity - (1 - a / 200)) > 0.1))
            fail(`ready ${r} ms, +${a} ms: t = ${s.t} (expected ≈ ${r + a}), opacity = ${s.opacity} (expected ≈ ${1 - a / 200})`);
          const png = await p.screenshot();
          if (a === 100) mid.set(r, png);
          cells.push({ label: `ready ${r} ms · +${a} ms`, png });
        }
        await p.clock.runFor(220 - elapsed);
        if (await p.locator(".start").count()) fail(`ready ${r} ms: start screen still there after 220 ms`);
        await p.close();
      }
      // No threshold: a boot at 149 ms and one at 151 ms look alike 100 ms into the hand-off — < 1 % of pixels
      // differ visibly (> 16/255; the fade's rAF phase alone shifts every pixel by a level or two).
      const d = await pixelDiff(mid.get(149)!, mid.get(151)!, 16);
      if (d < 0 || d > 0.01 * width * height) fail(`ready 149 vs 151 ms differ in ${d} px at +100 ms`);
      await sheet(`${out}spin.early-handoff.${size}.png`, `F · Spin-up — core ready early: 0 / 50 / 100 / 150 ms into the 200 ms hand-off, ${width}×${height}, ${theme}`, theme, 4, 300, cells);
      console.log(`  spin.early-handoff.${size}.png (149 vs 151 ms: ${d} px differ)`);
    }
    await ctx.close();
  }
}

/** Small sizes: F at the full form's floor — wordmark 160 px wide (BRAND.md §Sizes) — every 100 ms, at 2×. */
async function spinSmall(theme: (typeof THEMES)[number]) {
  const ctx = await browser.newContext({ viewport: { width: 280, height: 160 }, deviceScaleFactor: 2, reducedMotion: "no-preference" });
  const frames: { label: string; png: Buffer }[] = [];
  for (const t of STORY) {
    const page = await ctx.newPage();
    await open(page, `spin-small.${theme}.${t}`, `concept=spin&theme=${theme}&w=160&t=${t}`);
    frames.push({ label: `${t} ms`, png: await page.screenshot() });
    await page.close();
  }
  await ctx.close();
  await sheet(`${out}spin.small.${theme}.png`, `F · Spin-up at the full form's floor: wordmark 160 px wide, 2×, every 100 ms — ${theme}`, theme, 5, 280, frames, 2);
  console.log(`  spin.small.${theme}.png`);
}

/** The review sheet: A · Slice and F · Spin-up side by side at 1440×900, every 100 ms (A's CSS motion is set
 *  to each instant like the motion.* frames; F's frames are the storyboard's). */
async function compareSheet(theme: (typeof THEMES)[number]) {
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1, reducedMotion: "no-preference" });
  const f = spinFrames.get(`1440x900.${theme}`)!;
  const cells: { label: string; png: Buffer }[] = [];
  for (const [i, t] of STORY.entries()) {
    const page = await ctx.newPage();
    await open(page, `compare.${theme}.${t}`, `concept=slice&theme=${theme}`);
    await page.evaluate((at) => {
      for (const a of document.getAnimations()) {
        a.pause();
        a.currentTime = at;
      }
    }, t);
    cells.push({ label: `A · Slice — ${t} ms`, png: await page.screenshot() }, { label: `F · Spin-up — ${t} ms`, png: f[i]!.png });
    await page.close();
  }
  await ctx.close();
  await sheet(`${out}compare.slice-vs-spin.${theme}.png`, `A · Slice (left) vs F · Spin-up (right) — 1440×900, ${theme}, every 100 ms`, theme, 2, 560, cells);
  console.log(`  compare.slice-vs-spin.${theme}.png`);
}

/** Number of pixels that differ by more than `tol` (0–255, any channel) between two PNGs (decoded in the browser;
 *  −1 if their sizes differ). */
async function pixelDiff(a: Buffer, b: Buffer, tol = 0) {
  const page = await browser.newPage();
  const d = await page.evaluate(async ([a, b, tol]) => {
    const pixels = async (b64: string) => {
      const img = await createImageBitmap(await (await fetch(`data:image/png;base64,${b64}`)).blob());
      const g = new OffscreenCanvas(img.width, img.height).getContext("2d")!;
      g.drawImage(img, 0, 0);
      return g.getImageData(0, 0, img.width, img.height).data;
    };
    const [x, y] = await Promise.all([pixels(a), pixels(b)]);
    if (x.length !== y.length) return -1;
    let n = 0;
    for (let i = 0; i < x.length; i += 4)
      if (Math.max(Math.abs(x[i]! - y[i]!), Math.abs(x[i + 1]! - y[i + 1]!), Math.abs(x[i + 2]! - y[i + 2]!)) > tol) n++;
    return n;
  }, [a.toString("base64"), b.toString("base64"), tol] as const);
  await page.close();
  return d;
}

/** A labelled contact sheet of frames, `cols` per row, each `width` CSS px wide. */
async function sheet(path: string, title: string, theme: (typeof THEMES)[number], cols: number, width: number, cells: { label: string; png: Buffer }[], scale = 1) {
  const [bg, ink, line] = theme === "dark" ? ["#111317", "#ece6dc", "#2c3037"] : ["#e6e1d6", "#1a1d22", "#c9c3b6"];
  const page = await browser.newPage({ viewport: { width: cols * (width + 20) + 60, height: 400 }, deviceScaleFactor: scale });
  page.on("pageerror", (e) => failures.push(`${path}: ${e.message}`));
  await page.setContent(`<style>
    body { margin: 0; padding: 24px 30px; background: ${bg}; color: ${ink}; font: 13px system-ui, sans-serif; }
    h1 { font-size: 14px; font-weight: 600; margin: 0 0 16px; }
    .grid { display: grid; grid-template-columns: repeat(${cols}, ${width}px); gap: 14px 20px; }
    figure { margin: 0; display: grid; gap: 6px; } img { width: ${width}px; display: block; outline: 1px solid ${line}; }
    figcaption { opacity: .75; font-variant-numeric: tabular-nums; }
  </style><h1>${title}</h1><div class="grid">${cells
    .map((c) => `<figure><img src="data:image/png;base64,${c.png.toString("base64")}"><figcaption>${c.label}</figcaption></figure>`)
    .join("")}</div>`);
  await page.waitForFunction(() => [...document.images].every((i) => i.complete && i.naturalWidth > 0));
  await page.screenshot({ path, fullPage: true });
  await page.close();
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
