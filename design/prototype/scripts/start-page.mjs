// Shared by `pnpm screens` and `pnpm test:start`: opens the prototype as a cold start on Playwright's fake
// clock, so boot timers (src/boot.ts, the 1.2 s status) and F's requestAnimationFrame driver are deterministic.
// Time is paused at load and only moves with page.clock.runFor(). WAAPI (the hand-off fade) and the status
// line's CSS fade run on the document timeline, not the fake clock: every animation the app creates starts
// paused. A motion frame at an exact t is painted with paintAt.
import { fileURLToPath } from "node:url";
export async function openStart(browser, base, { query, width = 1440, height = 900, reducedMotion = "no-preference", onError }) {
  const ctx = await browser.newContext({ viewport: { width, height }, deviceScaleFactor: 1, reducedMotion });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => onError(`${query}: ${e.message}`));
  page.on("console", (m) => m.type() === "error" && onError(`${query}: console: ${m.text()}`));
  await page.addInitScript(() => {
    // the hand-off cross-fade is created paused, so a test controls when (and whether) it runs
    const animate = Element.prototype.animate;
    Element.prototype.animate = function (...args) {
      const a = animate.apply(this, args);
      a.pause();
      return a;
    };
    // has a start screen ever been in the DOM? (an instant boot must never show one)
    window.__startSeen = false;
    new MutationObserver(() => {
      if (document.querySelector(".start")) window.__startSeen = true;
    }).observe(document, { childList: true, subtree: true });
  });
  await page.clock.install({ time: 0 });
  await page.clock.pauseAt(10);
  await page.goto(`${base}?${query}`);
  return { page, close: () => ctx.close() };
}

/** F · Spin-up's pure module (design/brand/src/spin.ts), as the page's own dev server serves it. */
export const SPIN_MODULE = "/@fs" + fileURLToPath(new URL("../../brand/src/spin.ts", import.meta.url));

/** Paint F's frame at exactly `t` ms onto the start screen's SVG (the page's own rAF driver only advances with
 *  the fake clock, so the frame stays where it is put). Returns the frame's `morph` summary, or null. */
export async function paintAt(page, t) {
  await page.evaluate(
    async ({ mod, t }) => {
      const { frame, paint } = await import(mod);
      const svg = document.querySelector(".start svg");
      paint(svg, frame(t), { ink: "var(--start-ink)", accent: "var(--start-accent)" });
      document.querySelector(".start").dataset.t = String(t);
    },
    { mod: SPIN_MODULE, t },
  );
}
