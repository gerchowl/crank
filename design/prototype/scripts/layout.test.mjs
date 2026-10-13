// pnpm test:layout — layout invariants of DESIGN §16 that need a real window (#179):
//  - Today at 1440×900: Start is above the fold in every desk state (check-in open, injury cap, carried note) and the
//    desk card does not stretch past its actions (no empty band under "Why today", §16.2);
//  - Workouts: opening the library drawer at 1024, growing the window to ≥ 1280 px and shrinking it again leaves the
//    drawer closed (§16.3) — it must not reappear as a modal.
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";

const root = fileURLToPath(new URL("..", import.meta.url));
const server = await createServer({ root, logLevel: "warn", server: { port: 0 } });
await server.listen();
const base = server.resolvedUrls?.local[0];
const browser = await chromium.launch();
const results = [];
const check = (name, ok, detail = "") => results.push({ name, ok, detail });
const errors = [];
async function page(q, viewport) {
  const p = await browser.newPage({ viewport, reducedMotion: "reduce" });
  p.on("pageerror", (e) => errors.push(`${q}: ${e.message}`));
  await p.goto(base + q);
  await p.waitForTimeout(300);
  return p;
}

try {
  // 1. Start above the fold on the desk card, every desk state
  for (const scene of ["today", "today-health-cap", "today-checkin-edit", "today-wish-carried"]) {
    const p = await page(`?scene=${scene}`, { width: 1440, height: 900 });
    const m = await p.evaluate(() => {
      const start = [...document.querySelectorAll(".proposal-card.desk .actions button")].find((b) => /Start/.test(b.textContent ?? ""));
      const card = document.querySelector(".proposal-card.desk")?.getBoundingClientRect();
      const r = start?.getBoundingClientRect();
      // the card's last child (the hint under a disabled Start) ends the content: nothing may sit below it but padding
      const last = document.querySelector(".proposal-card.desk")?.lastElementChild?.getBoundingClientRect();
      return r && card && last ? { top: r.top, bottom: r.bottom, band: card.bottom - last.bottom, h: innerHeight } : null;
    });
    check(`Start fold: ${scene} — Start is inside 1440×900`, !!m && m.top >= 0 && m.bottom <= m.h, m ? `${Math.round(m.top)}–${Math.round(m.bottom)}` : "desk card or Start missing");
    check(`Start fold: ${scene} — no empty band under the card's content`, !!m && m.band <= 24, m ? `${Math.round(m.band)} px` : "");
    await p.close();
  }

  // 2. the library drawer across the 1280 px breakpoint
  const p = await page("?scene=workouts", { width: 1024, height: 768 });
  const open = async () => (await p.locator(".library[data-open]").count()) === 1;
  await p.locator(".lib-toggle").click();
  check("Resize: the drawer opens at 1024", (await open()) && (await p.locator(".lib-scrim").count()) === 1);
  await p.setViewportSize({ width: 1440, height: 900 });
  await p.waitForTimeout(150);
  check("Resize: growing to 1440 closes it", !(await open()));
  await p.setViewportSize({ width: 1024, height: 768 });
  await p.waitForTimeout(150);
  check("Resize: shrinking back to 1024 leaves it closed (no modal, editor not inert)", !(await open()) && (await p.locator(".lib-scrim").count()) === 0 && (await p.locator(".editor[inert]").count()) === 0);
  await p.locator(".lib-toggle").evaluate((b) => b.click()); // a stuck modal would intercept a pointer click
  check("Resize: ☰ Library still opens it afterwards", await open());
} finally {
  await browser.close();
  await server.close();
}
for (const r of results) console.log(`${r.ok ? "✓" : "✗"} ${r.name}${r.detail ? ` (${r.detail})` : ""}`);
if (errors.length) console.error(`page errors:\n${errors.join("\n")}`);
const failed = results.filter((r) => !r.ok).length;
console.log(`${results.length - failed}/${results.length} passed`);
if (failed || errors.length) process.exit(1);
