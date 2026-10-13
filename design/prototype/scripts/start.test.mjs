// pnpm test:start — the cold-start screen: F · Spin-up (docs/design/BRAND.md §F · Spin-up, issues #62, #138).
// Drives the prototype with Playwright on a fake clock and fails unless: an instant boot never shows the screen
// and a boot never waits on the motion; the hand-off makes the destination interactive (on top, focused) while
// the start screen is inert and cross-fades out, F's motion running on at 1:1 under the fade — for a boot at
// 100 / 149 / 151 / 300 ms alike (no snap, no rush, no freeze); a slow boot plays through and rests on the
// wordmark; reduced motion shows the static wordmark, pixel-identical to A · Slice, and removes the screen at
// once; the status line is timed from boot (1.2 s) with and without reduced motion; the motion stays inside
// the 1.2 s budget and its settled frame is A's; a failed boot shows the alert at once while the motion plays
// through and rests, and Try again does not restart it; the ring's slit never shows as a hairline (#142);
// a failed boot says what broke, what the app does and what to do (DESIGN P6).
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";
import { svg } from "../../brand/src/wordmark.ts";
import { SPIN_MODULE, openStart } from "./start-page.mjs";

/** A · Slice as the start screen paints it: the settled frame must equal it. */
const A_SVG = svg("slice", { ink: "var(--start-ink)", accent: "var(--start-accent)" }, "start");

const root = fileURLToPath(new URL("..", import.meta.url));
const server = await createServer({ root, logLevel: "warn", server: { port: 0 } });
await server.listen();
const base = server.resolvedUrls?.local[0];
const browser = await chromium.launch();
const results = [];
const errors = [];
const check = (name, ok, detail = "") => results.push({ name, ok, detail });
const open = (query, opts = {}) => openStart(browser, base, { query, onError: (e) => errors.push(e), ...opts });

/** What the page shows of F's motion: the painted t, and whether the DOM is at rest (A's paths only). */
const motion = (page) =>
  page.evaluate(() => {
    const start = document.querySelector(".start");
    if (!start) return { t: null, rest: null };
    const hidden = (sel) => [...start.querySelectorAll(sel)].every((e) => e.getAttribute("display") === "none");
    const letters = [...start.querySelectorAll(".sp-l")];
    return {
      t: start.dataset.t === undefined ? null : Number(start.dataset.t),
      letters: letters.length,
      rest:
        hidden(".sp-mark") &&
        hidden(".sp-morph") &&
        hidden(".sp-seam") &&
        hidden(".sp-uncut") &&
        letters.every((e) => !e.hasAttribute("transform") && !e.hasAttribute("opacity")) &&
        [...start.querySelectorAll(".sp-line")].every((e) => !e.hasAttribute("clip-path") && e.getAttribute("display") !== "none"),
    };
  });

const state = (page) =>
  page.evaluate(() => {
    const at = document.elementFromPoint(innerWidth / 2, innerHeight / 2);
    const start = document.querySelector(".start");
    return {
      start: !!start,
      inert: !!start?.inert,
      seen: window.__startSeen,
      status: document.querySelector(".start-status")?.textContent ?? null,
      alert: document.querySelector(".start-error")?.textContent ?? null,
      topIsDestination: !!at?.closest("[data-destination]"),
      focusInDestination: !!document.activeElement?.closest("[data-destination]"),
      destination: !!document.querySelector("[data-destination]"),
    };
  });

try {
  // 1. no artificial delay: a core that is ready at once never shows the start screen
  {
    const { page, close } = await open("theme=dark");
    await page.waitForSelector("[data-destination]");
    const s = await state(page);
    check("instant boot: destination at first paint, start screen never shown", s.destination && !s.seen && !s.start && s.alert === null);
    check("instant boot: focus is not moved", !s.focusInDestination);
    await close();
  }

  for (const reducedMotion of ["no-preference", "reduce"]) {
    const tag = reducedMotion === "reduce" ? "reduced motion" : "motion";

    // 2. a fast boot: hand-off the moment the core is ready — no minimum duration, no status
    {
      const { page, close } = await open("boot=300&theme=dark", { reducedMotion });
      await page.waitForSelector(".start svg");
      let s = await state(page);
      check(`${tag}: start screen while booting, destination not rendered`, s.start && !s.destination);
      await page.clock.runFor(300);
      s = await state(page);
      check(`${tag}: ready → destination on top and focused as the hand-off begins`, s.topIsDestination && s.focusInDestination);
      check(`${tag}: fast boot never shows the status line`, s.status === null);
      if (reducedMotion === "reduce") check(`${tag}: start screen removed at once`, !s.start);
      else {
        check(`${tag}: start screen inert during the fade`, s.start && s.inert);
        // F's motion runs on at 1:1 under the fade: not frozen, not cut, not fast-forwarded
        const m0 = await motion(page);
        await page.clock.runFor(96);
        const m1 = await motion(page);
        check(`${tag}: motion runs on at 1:1 under the fade`, m0.t !== null && m1.t - m0.t >= 80 && m1.t - m0.t <= 112, `${m0.t} → ${m1.t}`);
        // the destination takes input while the start screen is still fading: click the rail's History
        const hist = await page.getByRole("button", { name: "History" }).boundingBox();
        await page.mouse.click(hist.x + hist.width / 2, hist.y + hist.height / 2);
        check(`${tag}: destination takes a click during the fade`, (await page.getByRole("button", { name: "History" }).getAttribute("aria-current")) === "page");
        // the 200 ms cross-fade: half-way it is visibly translucent, then the screen is gone
        const fade = async (at) =>
          page.evaluate((t) => {
            const a = document.getAnimations().find((x) => x.effect?.target?.classList.contains("start"));
            if (!a) return null;
            a.currentTime = t;
            if (t >= 200) a.finish(); // a paused animation never "finishes" by itself
            return { dur: a.effect.getComputedTiming().duration, opacity: Number(getComputedStyle(document.querySelector(".start")).opacity) };
          }, at);
        const half = await fade(100);
        check(`${tag}: 200 ms linear cross-fade`, half?.dur === 200 && Math.abs(half.opacity - 0.5) < 0.01, JSON.stringify(half));
        await fade(200);
        // (waitForFunction polls on rAF, which the fake clock holds: poll from here, in real time)
        let gone = false;
        for (let i = 0; i < 40 && !gone; i++) {
          gone = !(await state(page)).start;
          if (!gone) await new Promise((r) => setTimeout(r, 25));
        }
        check(`${tag}: start screen removed when the fade ends`, gone);
      }
      await close();
    }

    // 3. the wordmark, the motion budget and the settled frame
    {
      const { page, close } = await open("boot=5000&theme=light", { reducedMotion });
      await page.waitForSelector(".start svg");
      if (reducedMotion === "reduce") {
        const m = await motion(page);
        const anims = await page.evaluate(() => document.getAnimations().length);
        check(`${tag}: static wordmark (final frame, nothing driven, no animation)`, m.letters === 5 && m.rest === true && m.t === null && anims === 0, JSON.stringify(m));
      } else {
        let m = await motion(page);
        check(`${tag}: starts at t = 0 with the mark spinning, the word not yet shown`, m.t === 0 && m.rest === false, JSON.stringify(m));
        const seen = [];
        for (let t = 0; t <= 1200 && seen.length < 100; t += 16) {
          await page.clock.runFor(16);
          m = await motion(page);
          seen.push(m);
          if (m.rest) break;
        }
        const first = seen.findIndex((x) => x.rest);
        const at = first < 0 ? Infinity : seen[first].t;
        check(`${tag}: launch motion at rest by ≤ 1.1 s (budget 1.2 s)`, at <= 1100, `${at} ms`);
        check(`${tag}: motion is monotonic, painted t never goes back`, seen.every((x, i) => i === 0 || x.t >= seen[i - 1].t));
      }
      // the settled frame (and reduced motion) is A · Slice, pixel for pixel
      await page.clock.runFor(60); // still before the 1.2 s status line
      const settled = await page.screenshot();
      await page.evaluate((a) => (document.querySelector(".start .wm").innerHTML = a), A_SVG);
      const slice = await page.screenshot();
      check(`${tag}: settled frame is A · Slice, pixel-identical`, settled.equals(slice));
      await close();
    }

    // 4. slow boot: the status is timed from boot, same with and without reduced motion
    {
      const { page, close } = await open("boot=5000&theme=dark", { reducedMotion });
      await page.waitForSelector(".start svg");
      await page.clock.runFor(1190);
      check(`${tag}: no status before 1.2 s`, (await state(page)).status === null);
      await page.clock.runFor(20);
      const s = await state(page);
      check(`${tag}: status “Starting crank…” after 1.2 s`, s.status === "Starting crank…");
      check(`${tag}: status is role=status`, (await page.locator('.start-status[role="status"]').count()) === 1);
      await page.clock.runFor(4000);
      check(`${tag}: slow boot hands off too, status gone`, (await state(page)).status === null && (await state(page)).topIsDestination);
      await close();
    }
  }

  // 4b. fast boot (BRAND.md §Boot behaviours (a)): ready at ≤ 150 ms and just after it look the same — the hand-off
  // begins at once and the motion runs on at 1:1 under the fade: no cut to the wordmark at ≤ 150 ms, no
  // fast-forward after it (round 1 of #131 had a snap at 149 ms and a rush at 151 ms)
  for (const ready of [0, 100, 149, 151]) {
    const { page, close } = await open(`boot=${ready}&theme=dark`);
    await page.waitForSelector(".start svg");
    await page.clock.runFor(ready + 1);
    const m0 = await motion(page);
    const s0 = await state(page);
    check(`fast boot ${ready} ms: hand-off begins at once, motion still spinning (not cut to the wordmark)`, s0.topIsDestination && s0.focusInDestination && s0.inert && m0.rest === false, JSON.stringify(m0));
    await page.clock.runFor(96);
    const m1 = await motion(page);
    check(`fast boot ${ready} ms: motion advanced 1:1 under the fade (no freeze, no rush)`, m1.t - m0.t >= 80 && m1.t - m0.t <= 112, `${m0.t} → ${m1.t}`);
    await close();
  }

  // 4c. slow boot (b): the sequence plays through and rests on the wordmark, never held, never gated on ready
  {
    const { page, close } = await open("boot=5000&theme=dark");
    await page.waitForSelector(".start svg");
    await page.clock.runFor(700);
    check("slow boot: mid-motion at 700 ms (not waiting for the core)", (await motion(page)).rest === false);
    await page.clock.runFor(500);
    const m = await motion(page);
    check("slow boot: rests on the wordmark with the screen still up", m.rest === true && (await state(page)).start, JSON.stringify(m));
    await page.clock.runFor(2000);
    check("slow boot: stays at rest until the core is ready (no loop, no hold)", (await motion(page)).rest === true && (await motion(page)).t === 1080);
    await close();
  }

  // 4d. the ring's slit (#142): the two faces of the ring part from one radius at 3 o'clock. No hairline may show
  // before they are visibly (1 px) apart; once they are, the gap only widens. F is painted frame by frame (1 ms)
  // through the page's own module and the slit is measured on the pixels at three places along it. (At 60 Hz a
  // 1 ms event is seen in one frame of four, hence the 1 ms steps.)
  for (const [width, height, theme] of [
    [1440, 900, "dark"],
    [1024, 768, "light"],
  ]) {
    const { page, close } = await open(`boot=99999&theme=${theme}`, { width, height });
    await page.waitForSelector(".start svg");
    await page.evaluate(async (mod) => (window.__spin = await import(mod)), SPIN_MODULE);
    const tag = `${width}×${height} ${theme}`;
    for (const k of [12, 24, 36]) {
      const series = [];
      for (let t = 420; t <= 466; t++) {
        // paint t, and report where face vertex pair k sits on the screen and how far apart the faces are (px)
        const at = await page.evaluate(
          ({ t, k }) => {
            const { frame, paint } = window.__spin;
            const svg = document.querySelector(".start svg");
            const c = { ink: "var(--start-ink)", accent: "var(--start-accent)" };
            const nums = (d) => d.match(/-?\d+\.?\d*/g).map(Number);
            const pair = (f) => {
              const v = nums(f.morph.d);
              const p = (i) => [v[2 * i], v[2 * i + 1]];
              return [p(320 + k), p((320 + 48 + 200 + 48 - k) % (v.length / 2))];
            };
            const f = frame(t);
            paint(svg, f, c);
            const [a, b] = pair(f);
            const r = svg.getBoundingClientRect();
            const s = r.width / svg.viewBox.baseVal.width;
            const x = (u) => r.left + (u - svg.viewBox.baseVal.x) * s;
            const y = (u) => r.top + (u - svg.viewBox.baseVal.y) * s;
            return { x: x((a[0] + b[0]) / 2), y: y((a[1] + b[1]) / 2), gap: Math.hypot(a[0] - b[0], a[1] - b[1]) * s };
          },
          { t, k },
        );
        const shot = await page.screenshot({ clip: { x: Math.round(at.x) - 3, y: Math.round(at.y) - 6, width: 7, height: 13 } });
        // darkest departure of the row-averaged profile from the band's own colour (0 = solid, 1 = background)
        const dip = await page.evaluate(async (b64) => {
          const bmp = await createImageBitmap(await (await fetch(`data:image/png;base64,${b64}`)).blob());
          const g = new OffscreenCanvas(bmp.width, bmp.height).getContext("2d");
          g.drawImage(bmp, 0, 0);
          const d = g.getImageData(0, 0, bmp.width, bmp.height).data;
          const rows = [];
          for (let y = 0; y < bmp.height; y++) {
            let sum = 0;
            for (let x = 0; x < bmp.width; x++) sum += 0.3 * d[(y * bmp.width + x) * 4] + 0.59 * d[(y * bmp.width + x) * 4 + 1] + 0.11 * d[(y * bmp.width + x) * 4 + 2];
            rows.push(sum / bmp.width);
          }
          const ref = (rows[0] + rows[rows.length - 1]) / 2;
          return Math.max(...rows.map((v) => Math.abs(v - ref))) / 255;
        }, shot.toString("base64"));
        series.push({ t, gap: at.gap, dip });
      }
      const hair = series.filter((x) => x.gap < 1 && x.dip > 0.02);
      check(`slit ${tag} @${k}: no hairline while the faces are < 1 px apart`, hair.length === 0, hair.length ? `t=${hair[0].t} ms: gap ${hair[0].gap.toFixed(2)} px, dip ${hair[0].dip.toFixed(2)}` : "");
      const open1 = series.findIndex((x) => x.gap >= 1);
      const wider = series.slice(open1).every((x, i, all) => i === 0 || x.dip >= all[i - 1].dip - 0.03);
      check(`slit ${tag} @${k}: once 1 px apart the gap only widens (no close/reopen)`, open1 > 0 && wider);
    }
    await close();
  }

  // 5. boot failure (DESIGN P6): what broke, what the app does, what to do — replacing the status line
  {
    const { page, close } = await open("boot=fail:2000&theme=dark");
    await page.waitForSelector(".start svg");
    await page.clock.runFor(1500);
    check("failure: slow boot shows the status first", (await state(page)).status === "Starting crank…");
    await page.clock.runFor(600);
    const s = await state(page);
    check("failure: error replaces the status line", s.status === null && s.alert !== null);
    check("failure: F's motion is not cut — it played through and rests on the wordmark", (await motion(page)).rest === true);
    check("failure: says what broke", /ride core didn’t come up/.test(s.alert ?? ""));
    check("failure: says what the app did", /not touched/.test(s.alert ?? ""));
    check("failure: says what to do", /Try again/.test(s.alert ?? "") && /quit and reopen/.test(s.alert ?? ""));
    check("failure: role=alert, start screen stays, not inert", (await page.locator('.start-error[role="alert"]').count()) === 1 && s.start && !s.inert);
    await page.getByRole("button", { name: "Try again" }).click();
    // right after the click, and a frame or two later: a restarted motion would be back at t ≈ 0 (not at rest)
    await page.clock.runFor(34);
    const kept = await motion(page);
    check("failure: Try again does not restart the motion (t stays at 1080, at rest)", kept.rest === true && kept.t === 1080, JSON.stringify(kept));
    await page.clock.runFor(1190 - 34);
    let r = await state(page);
    check("failure: Try again starts a fresh boot with its own status clock (none before 1.2 s)", r.alert === null && r.status === null);
    await page.clock.runFor(20);
    check("failure: …status after 1.2 s", (await state(page)).status === "Starting crank…");
    await close();
  }
  // 5b. the core fails *during* the motion: the alert shows at once, the motion is not cut — it keeps advancing
  // past the failure and rests on the wordmark (BRAND.md §Boot behaviours, boot failure)
  {
    const { page, close } = await open("boot=fail:300&theme=dark");
    await page.waitForSelector(".start svg");
    await page.clock.runFor(310);
    const at = await motion(page);
    const s = await state(page);
    check("failure mid-motion: alert shown at once, motion still running", s.alert !== null && at.rest === false && at.t > 0 && at.t < 1080, JSON.stringify(at));
    await page.clock.runFor(96);
    const later = await motion(page);
    check("failure mid-motion: painted t keeps advancing 1:1 past the failure (not cut, not frozen)", later.t - at.t >= 80 && later.t - at.t <= 112, `${at.t} → ${later.t}`);
    await page.clock.runFor(900);
    const end = await motion(page);
    check("failure mid-motion: plays through and rests on the wordmark, alert still up", end.rest === true && end.t === 1080 && (await state(page)).alert !== null, JSON.stringify(end));
    await close();
  }
  {
    const { page, close } = await open("boot=fail&theme=light");
    await page.waitForSelector(".start svg");
    await page.clock.runFor(1);
    check("failure: an immediate failure shows at once (no wait for 1.2 s)", (await state(page)).alert !== null);
    await close();
  }
} finally {
  await browser.close();
  await server.close();
}
for (const r of results) console.log(`${r.ok ? "✓" : "✗"} ${r.name}${r.detail ? ` (${r.detail})` : ""}`);
if (errors.length) console.error(`page errors:\n${errors.join("\n")}`);
const failed = results.filter((r) => !r.ok).length;
console.log(`${results.length - failed}/${results.length} passed`);
if (failed || errors.length) process.exit(1);
