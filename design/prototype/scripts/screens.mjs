// pnpm screens — renders every scene (src/scenes.ts) at each window size and theme into
// docs/design/screens/<scene>.<w>x<h>.<theme>.png (DESIGN §12). Scenes are seeded and frozen,
// so the output is deterministic. Fails on page errors, scene errors and horizontal overflow
// (ride scenes: any overflow — the ride screen must fit the window, #7), and on any [data-fit]
// element whose text is clipped (reserved rows never wrap or cut text, DESIGN §6.1). Runs the
// contrast check first (DESIGN §6.3), and checks every rendered chart mark's composited contrast.
// Saddle scenes (Today after the check-in, §15.2) are captured as the viewport, not the full page, and fail
// when the wish, the session, its reason or the actions leave the viewport or fall below their type tier.
// Above-the-fold elements ([data-above-fold]: the saddle card, the desk card's outcome and session, the ride's "next"
// label on the road, DESIGN §16) fail when they leave the viewport in either direction. Ride geometry is fixed (P4,
// #10): every [data-geom] box (status-bar groups, hero rows, side slots, the coach lane and its reserved rows, the road
// rows) must match the first ride scene at the same size, in every phase and state.
// `SCENES=a,b pnpm screens` renders only those scenes (development; keeps the other PNGs and skips the start screen).
// Also renders the cold-start screen (start-*.png: frames of the launch motion, reduced motion, slow boot,
// hand-off, boot failure; BRAND.md §Start screen) on the fake clock of scripts/start-page.mjs.
import { execFileSync } from "node:child_process";
import { mkdirSync, readdirSync, rmSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";
import { freezeMotion, openStart } from "./start-page.mjs";

const root = fileURLToPath(new URL("..", import.meta.url));
const out = fileURLToPath(new URL("../../../docs/design/screens/", import.meta.url));
const SIZES = [
  [1440, 900],
  [1024, 768],
];
const THEMES = ["dark", "light"];

execFileSync(process.execPath, [fileURLToPath(new URL("./contrast.mjs", import.meta.url))], { stdio: "inherit" });
mkdirSync(out, { recursive: true });
const only = process.env.SCENES ? new Set(process.env.SCENES.split(",")) : null;
if (!only) for (const f of readdirSync(out)) if (f.endsWith(".png")) rmSync(out + f);

const server = await createServer({ root, logLevel: "warn", server: { port: 0 } });
await server.listen();
const base = server.resolvedUrls?.local[0];
if (!base) throw new Error("vite did not report a local URL");
const browser = await chromium.launch();
const failures = [];

try {
  const lister = await browser.newPage();
  await lister.goto(`${base}?list-scenes=1`);
  const ids = JSON.parse((await lister.locator("body").getAttribute("data-scenes")) ?? "[]").filter((id) => !only || only.has(id));
  await lister.close();
  if (!ids.length) throw new Error("no scenes listed");

  for (const [width, height] of SIZES) {
    const ctx = await browser.newContext({ viewport: { width, height }, deviceScaleFactor: 1, reducedMotion: "reduce" });
    for (const theme of THEMES) {
      /** the first ride scene's [data-geom] boxes at this size and theme: every other ride scene must match them */
      let geomRef = null;
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
          // the saddle proposal (DESIGN §15.2): wish, session, one reason and the actions inside the viewport, at T3+
          saddle: !!document.querySelector("[data-saddle]"),
          belowFold: [...document.querySelectorAll("[data-above-fold]")]
            .filter((el) => {
              const r = el.getBoundingClientRect();
              return r.bottom > innerHeight || r.top < 0 || r.left < 0 || r.right > innerWidth;
            })
            .map((el) => {
              const r = el.getBoundingClientRect();
              return `"${el.textContent?.trim().slice(0, 40)}" (${Math.round(r.left)},${Math.round(r.top)}–${Math.round(r.right)},${Math.round(r.bottom)} in ${innerWidth}×${innerHeight})`;
            }),
          geom: [...document.querySelectorAll(".ride [data-geom]")].map((el, i, all) => {
            const r = el.getBoundingClientRect();
            const name = el.getAttribute("data-geom");
            const n = all.slice(0, i).filter((e) => e.getAttribute("data-geom") === name).length;
            return [`${name}#${n}`, [r.left, r.top, r.width, r.height].map((v) => Math.round(v * 2) / 2)];
          }),
          small: [...document.querySelectorAll("[data-tier]")]
            .filter((el) => {
              const t3 = Math.min(36, Math.max(27, 0.037 * innerHeight));
              const min = el.getAttribute("data-tier") === "t3plus" ? Math.min(44, Math.max(34, 0.045 * innerHeight)) : t3;
              return parseFloat(getComputedStyle(el).fontSize) < min - 0.5;
            })
            .map((el) => `"${el.textContent?.trim().slice(0, 40)}" ${getComputedStyle(el).fontSize}`),
          clipped: [...document.querySelectorAll("[data-fit]")]
            .filter((el) => el.scrollWidth > el.clientWidth + 1 || el.scrollHeight > el.clientHeight + 1)
            .map((el) => `"${el.textContent?.trim().slice(0, 50)}" (${el.scrollWidth}×${el.scrollHeight} in ${el.clientWidth}×${el.clientHeight})`),
        }));
        for (const c of m.clipped) failures.push(`${name}: clipped text ${c}`);
        for (const c of m.belowFold) failures.push(`${name}: above-the-fold element outside the viewport ${c}`);
        if (m.ride && m.geom.length) {
          if (!geomRef) geomRef = { id, boxes: new Map(m.geom) };
          for (const [k, box] of m.geom) {
            const ref = geomRef.boxes.get(k);
            if (ref && ref.some((v, i) => Math.abs(v - box[i]) > 0.5)) failures.push(`${name}: ride geometry moved — ${k} at ${box.join(",")} vs ${ref.join(",")} in ${geomRef.id}`);
          }
        }
        for (const c of m.small) failures.push(`${name}: saddle text below its tier ${c}`);
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
        await page.screenshot({ path: `${out}${name}.png`, fullPage: !m.ride && !m.saddle, animations: "disabled" });
        console.log(`  ${name}.png`);
        await page.close();
      }
    }
    await ctx.close();
  }
  if (!only) await startScreens();
} finally {
  await browser.close();
  await server.close();
}

/** The cold-start screen, every state at each size and theme (BRAND.md §Start screen; checks in start.test.mjs). */
async function startScreens() {
  const MOTION_FRAMES = [0, 300, 750, 1200]; // ms into the launch motion; 1200 = the budget, all at rest
  const STATES = [
    ...MOTION_FRAMES.map((t) => ({ id: `start-motion-${String(t).padStart(4, "0")}ms`, query: "boot=5000", async prepare(page) { await freezeMotion(page, t); } })),
    { id: "start-reduced-motion", query: "boot=5000", reducedMotion: "reduce", async prepare() {} },
    { id: "start-slow-boot", query: "boot=5000", reducedMotion: "reduce", async prepare(page) { await page.clock.runFor(1250); } },
    {
      id: "start-handoff-100ms",
      query: "boot=600",
      async prepare(page) {
        await page.clock.runFor(600); // the core is ready: destination live underneath, start screen frozen and fading
        await freezeMotion(page, 600);
        await page.evaluate(() => {
          const fade = document.getAnimations().find((a) => a.effect?.target?.classList.contains("start"));
          fade.currentTime = 100;
        });
      },
    },
    { id: "start-failed", query: "boot=fail:2000", reducedMotion: "reduce", async prepare(page) { await page.clock.runFor(2100); } },
  ];
  for (const [width, height] of SIZES) {
    for (const theme of THEMES) {
      for (const st of STATES) {
        const name = `${st.id}.${width}x${height}.${theme}`;
        const { page, close } = await openStart(browser, base, { query: `${st.query}&theme=${theme}`, width, height, reducedMotion: st.reducedMotion ?? "no-preference", onError: (e) => failures.push(`${name}: ${e}`) });
        await page.waitForSelector(".start svg");
        await st.prepare(page);
        const m = await page.evaluate(() => {
          const r = document.querySelector(".start .wm svg").getBoundingClientRect();
          return { sw: document.documentElement.scrollWidth, l: r.left, r: r.right, t: r.top, b: r.bottom };
        });
        if (m.sw > width) failures.push(`${name}: horizontal overflow (${m.sw} > ${width})`);
        if (m.l < 0 || m.r > width || m.t < 0 || m.b > height) failures.push(`${name}: wordmark leaves the window`);
        await page.screenshot({ path: `${out}${name}.png` });
        console.log(`  ${name}.png`);
        await close();
      }
    }
  }
}
if (failures.length) {
  console.error(`\n${failures.length} problem(s):\n${failures.join("\n")}`);
  process.exit(1);
}
