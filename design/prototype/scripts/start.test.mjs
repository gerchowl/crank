// pnpm test:start — the cold-start screen (docs/design/BRAND.md §Start screen, issue #62). Drives the
// prototype with Playwright on a fake clock and fails unless: an instant boot never shows the screen and
// a boot never waits on the motion; the hand-off makes the destination interactive (on top, focused) while
// the start screen is inert and cross-fades out; reduced motion shows the static wordmark and removes the
// screen at once; the status line is timed from boot (1.2 s) with and without reduced motion; the launch
// motion stays inside 1.2 s; a failed boot says what broke, what the app does and what to do (DESIGN P6).
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";
import { freezeMotion, openStart } from "./start-page.mjs";

const root = fileURLToPath(new URL("..", import.meta.url));
const server = await createServer({ root, logLevel: "warn", server: { port: 0 } });
await server.listen();
const base = server.resolvedUrls?.local[0];
const browser = await chromium.launch();
const results = [];
const errors = [];
const check = (name, ok, detail = "") => results.push({ name, ok, detail });
const open = (query, opts = {}) => openStart(browser, base, { query, onError: (e) => errors.push(e), ...opts });

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

    // 3. the wordmark and the motion budget
    {
      const { page, close } = await open("boot=5000&theme=light", { reducedMotion });
      await page.waitForSelector(".start svg");
      const n = await page.evaluate(() => document.querySelectorAll(".start .wm-l").length);
      const longest = await freezeMotion(page, 0);
      if (reducedMotion === "reduce") {
        const rest = await page.evaluate(() => {
          const l = document.querySelector(".start .wm-l");
          const s = document.querySelector(".start .wm-speed");
          return { opacity: getComputedStyle(l).opacity, clip: getComputedStyle(s).clipPath, anims: document.getAnimations().length };
        });
        check(`${tag}: static wordmark (final frame, no animation)`, n === 5 && rest.opacity === "1" && rest.clip === "none" && longest === 0 && rest.anims === 0, JSON.stringify(rest));
      } else check(`${tag}: launch motion ≤ 1.2 s`, longest > 0 && longest <= 1200, `${longest} ms`);
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

  // 5. boot failure (DESIGN P6): what broke, what the app does, what to do — replacing the status line
  {
    const { page, close } = await open("boot=fail:2000&theme=dark");
    await page.waitForSelector(".start svg");
    await page.clock.runFor(1500);
    check("failure: slow boot shows the status first", (await state(page)).status === "Starting crank…");
    await page.clock.runFor(600);
    const s = await state(page);
    check("failure: error replaces the status line", s.status === null && s.alert !== null);
    check("failure: says what broke", /ride core didn’t come up/.test(s.alert ?? ""));
    check("failure: says what the app did", /not touched/.test(s.alert ?? ""));
    check("failure: says what to do", /Try again/.test(s.alert ?? "") && /quit and reopen/.test(s.alert ?? ""));
    check("failure: role=alert, start screen stays, not inert", (await page.locator('.start-error[role="alert"]').count()) === 1 && s.start && !s.inert);
    await page.getByRole("button", { name: "Try again" }).click();
    await page.clock.runFor(1190);
    let r = await state(page);
    check("failure: Try again starts a fresh boot with its own status clock (none before 1.2 s)", r.alert === null && r.status === null);
    await page.clock.runFor(20);
    check("failure: …status after 1.2 s", (await state(page)).status === "Starting crank…");
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
