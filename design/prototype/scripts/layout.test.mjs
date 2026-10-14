// pnpm test:layout — layout invariants of DESIGN §16 that need a real window (#179):
//  - Today at 1440×900: Start is above the fold in every desk state (check-in open, injury cap, carried note) and the
//    desk card does not stretch: nothing but the actions' own margin and padding sits between the card's last content
//    row and its actions (no empty band above Start, §16.2);
//  - Workouts: opening the library drawer at 1024, growing the window to ≥ 1280 px and shrinking it again leaves the
//    drawer closed (§16.3) — it must not reappear as a modal, also when the window grew while another place was showing.
// Waits are on observable state (the page painted, a media query matching, the drawer's attribute), never on time.
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
  await p.waitForFunction(() => document.getElementById("root")?.childElementCount && !document.querySelector("[data-pending]"));
  await p.evaluate(() => document.fonts.ready);
  return p;
}
/** `.proposal-card .actions { margin-top: 10px }` in app.css: the one designed space above the actions. */
const ACTIONS_MARGIN = 10;
/** Resize and wait until the browser reports the breakpoint on the new side and has delivered the change event. */
async function resize(p, width, height) {
  await p.setViewportSize({ width, height });
  await p.waitForFunction((w) => matchMedia("(min-width: 1280px)").matches === w >= 1280, width);
  // media-query change events are delivered in the frame's rendering update, before its animation-frame callbacks
  await p.evaluate(() => new Promise((r) => requestAnimationFrame(r)));
}

try {
  // 1. Start above the fold on the desk card, every desk state
  for (const scene of ["today", "today-health-cap", "today-checkin-edit", "today-wish-carried"]) {
    const p = await page(`?scene=${scene}`, { width: 1440, height: 900 });
    const m = await p.evaluate((actionsMargin) => {
      const card = document.querySelector(".proposal-card.desk");
      const start = [...(card?.querySelectorAll(".actions button") ?? [])].find((b) => /Start/.test(b.textContent ?? ""));
      const actions = card?.querySelector(":scope > .actions");
      const before = actions?.previousElementSibling;
      const r = start?.getBoundingClientRect();
      if (!r || !actions || !before) return null;
      // the band: from the row above the actions to the actions' top, against what separates them by design (the
      // actions' margin-top, the row's margin-bottom, the card's row gap; flex items do not collapse). Not the computed
      // margin-top: on a stretched card `margin-top: auto` resolves to the band itself.
      const gap = actions.getBoundingClientRect().top - before.getBoundingClientRect().bottom;
      return { top: r.top, bottom: r.bottom, gap, designed: actionsMargin + parseFloat(getComputedStyle(before).marginBottom) + parseFloat(getComputedStyle(card).rowGap), h: innerHeight };
    }, ACTIONS_MARGIN);
    check(`Start fold: ${scene} — Start is inside 1440×900`, !!m && m.top >= 0 && m.bottom <= m.h, m ? `${Math.round(m.top)}–${Math.round(m.bottom)}` : "desk card or Start missing");
    // 1 px of slack: the row above ends on a fractional line box, the margins and the gap are whole pixels
    check(`Start fold: ${scene} — no empty band above the actions`, !!m && m.gap <= m.designed + 1, m ? `${Math.round(m.gap)} px, designed ${m.designed} px` : "");
    await p.close();
  }

  // 2. the library drawer across the 1280 px breakpoint
  const p = await page("?scene=workouts", { width: 1024, height: 768 });
  const open = async () => (await p.locator(".library[data-open]").count()) === 1;
  await p.locator(".lib-toggle").click();
  check("Resize: the drawer opens at 1024", (await open()) && (await p.locator(".lib-scrim").count()) === 1);
  await resize(p, 1440, 900);
  check("Resize: growing to 1440 closes it", !(await open()));
  await resize(p, 1024, 768);
  check("Resize: shrinking back to 1024 leaves it closed (no modal, editor not inert)", !(await open()) && (await p.locator(".lib-scrim").count()) === 0 && (await p.locator(".editor[inert]").count()) === 0);
  await p.locator(".lib-toggle").evaluate((b) => b.click()); // a stuck modal would intercept a pointer click
  check("Resize: ☰ Library still opens it afterwards", await open());
  await p.close();

  // 3. the same, with the crossing happening while another place is showing (the state outlives the view)
  const q = await page("?scene=workouts", { width: 1024, height: 768 });
  await q.locator(".lib-toggle").click();
  check("Across views: the drawer opens at 1024", (await q.locator(".library[data-open]").count()) === 1);
  // the open drawer's scrim covers the rail, so leave by a DOM click: any non-pointer navigation (a shortcut, an alert)
  await q.locator("nav.rail button", { hasText: "Today" }).evaluate((b) => b.click());
  await q.waitForSelector(".today");
  await resize(q, 1440, 900);
  await resize(q, 1024, 768);
  await q.locator("nav.rail button", { hasText: "Workouts" }).click();
  await q.waitForSelector(".library", { state: "attached" }); // hidden while the drawer is closed
  check(
    "Across views: back on Workouts at 1024 after growing and shrinking elsewhere — no drawer, no modal",
    (await q.locator(".library[data-open]").count()) === 0 && (await q.locator(".lib-scrim").count()) === 0 && (await q.locator(".editor[inert]").count()) === 0,
  );
} finally {
  await browser.close();
  await server.close();
}
for (const r of results) console.log(`${r.ok ? "✓" : "✗"} ${r.name}${r.detail ? ` (${r.detail})` : ""}`);
if (errors.length) console.error(`page errors:\n${errors.join("\n")}`);
const failed = results.filter((r) => !r.ok).length;
console.log(`${results.length - failed}/${results.length} passed`);
if (failed || errors.length) process.exit(1);
