// Shared by `pnpm screens` and `pnpm test:start`: opens the prototype as a cold start on Playwright's fake
// clock, so boot timers (src/boot.ts, the 1.2 s status) are deterministic. Time is paused at load and only
// moves with page.clock.runFor(). CSS launch motion and WAAPI run on the document timeline, not the fake
// clock, so they are stepped by hand (freezeMotion): every animation the app creates starts paused.
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

/** Put the launch motion at `t` ms (the document timeline is not the fake clock) and hold it there. */
export function freezeMotion(page, t) {
  return page.evaluate((at) => {
    let longest = 0;
    for (const a of document.getAnimations()) {
      if (a.effect?.target?.classList?.contains("start")) continue; // the cross-fade, if any
      const end = a.effect?.getComputedTiming().endTime;
      if (typeof end === "number") longest = Math.max(longest, end);
      a.pause();
      a.currentTime = at;
    }
    return longest;
  }, t);
}
