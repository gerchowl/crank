// pnpm test:safety — the medical stop is a global invariant (DESIGN §5.8). Drives the prototype with
// Playwright and fails unless: every start path is blocked under a stop (only the ticked free ride
// starts), the daily re-check asks every red flag with no default, and the in-ride symptom action
// stops the workout and sets the stop. The planner wish (DESIGN §15.2) never overrides safety: no wish
// under a stop, no cell above the injury cap can be chosen or reached, and only readiness deviations offer
// "ride my wish". The cap is on actual power: non-ERG formats are not choices under it, a workout above it is
// refused at start, and power above it in a ride is flagged and logged. The remote reaches every wish cell.
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
async function page(q) {
  const p = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  p.on("pageerror", (e) => errors.push(`${q}: ${e.message}`));
  await p.goto(base + q);
  await p.waitForTimeout(300);
  return p;
}
const blocked = async (p) => (await p.locator('[aria-label="Ride blocked by the medical stop"]').count()) === 1;
const atReady = async (p) => (await p.locator('[aria-label="Ready check"]').count()) === 1;

try {
  // 1. every start path under a stop
  let p = await page("?stop=1");
  check("Today: no proposal under a stop", (await p.locator('[aria-label="Today\'s proposal"]').count()) === 0 && (await p.locator(".medical-stop").count()) === 1);
  await p.getByRole("button", { name: "Workouts" }).click();
  await p.getByRole("button", { name: "Ride this" }).click();
  check("Workouts → Ride this is blocked", await blocked(p));
  await p.getByRole("button", { name: "History" }).click();
  await p.getByRole("button", { name: "Ride again" }).first().click();
  check("History → Ride again is blocked", await blocked(p));
  p = await page("?scene=resume&stop=1");
  await p.getByRole("button", { name: /Resume/ }).click();
  check("Resume → Resume ride is blocked", await blocked(p));
  p = await page("?scene=resume-stale&stop=1");
  await p.getByRole("button", { name: "Ride the rest as a new ride" }).click();
  check("Resume → Ride the rest is blocked", await blocked(p));
  // the only exception: the deliberate, ticked free ride
  p = await page("?stop=1");
  await p.getByRole("button", { name: "Ride without a plan…" }).click();
  const startFree = p.getByRole("button", { name: "Start free ride" });
  check("Free ride needs the tick", await startFree.isDisabled());
  await p.getByRole("checkbox").check();
  await startFree.click();
  check("Free ride starts (the only exception)", (await atReady(p)) && (await p.getByText("Free ride", { exact: false }).count()) > 0);

  // 2. the daily re-check: every red flag, no default
  p = await page("");
  const rows = p.locator('.redflag-check [role="radiogroup"]');
  check("Re-check asks all 5 red flags", (await rows.count()) === 5, `${await rows.count()} rows`);
  check("Re-check has no default answer", (await p.locator('.redflag-check [aria-checked="true"]').count()) === 0);
  check("Re-check can't be submitted half-answered", await p.getByRole("button", { name: /Answer all 5/ }).isDisabled());
  for (let i = 0; i < 5; i++) await rows.nth(i).getByRole("radio", { name: i === 3 ? "yes" : "no" }).click(); // "unusual breathlessness" = yes
  await p.getByRole("button", { name: "Done" }).click();
  check("A 'yes' on any flag (breathlessness) sets the stop", (await p.locator(".medical-stop").count()) === 1);
  await p.getByRole("button", { name: "Workouts" }).click();
  await p.getByRole("button", { name: "Ride this" }).click();
  check("…and then Workouts is blocked too", await blocked(p));
  // a start without today's check goes through the same re-check first
  p = await page("");
  await p.getByRole("button", { name: "Workouts" }).click();
  await p.getByRole("button", { name: "Ride this" }).click();
  const gateRows = p.locator('.redflag-check [role="radiogroup"]');
  check("Workouts start without today's check → re-check first", (await gateRows.count()) === 5 && !(await atReady(p)));
  for (let i = 0; i < 5; i++) await gateRows.nth(i).getByRole("radio", { name: "no" }).click();
  await p.getByRole("button", { name: "Done" }).click();
  check("All 'no' → the ride reaches the Ready check", await atReady(p));

  // 3. in-ride symptom: its own binding, and the End dialog
  p = await page("?scene=ride-threshold");
  await p.waitForTimeout(1600);
  await p.keyboard.press("!");
  check("Symptom key stops the workout", (await p.locator('[aria-label="Workout stopped for a symptom"]').count()) === 1);
  p = await page("?scene=ride-threshold");
  await p.keyboard.press("Escape");
  check("End dialog focuses Keep riding", (await p.evaluate(() => document.activeElement?.textContent ?? "")).includes("Keep riding"));
  await p.getByRole("button", { name: /Stopping because of chest pain/ }).click();
  check("End dialog → symptom stops the workout", (await p.locator('[aria-label="Workout stopped for a symptom"]').count()) === 1);
  await p.getByRole("button", { name: /Save and close the ride/ }).click();
  await p.getByRole("button", { name: "Today" }).click();
  check("…and sets the medical stop (Today shows the stop)", (await p.locator(".medical-stop").count()) === 1);

  // 4. the wish never overrides safety; the cap is on actual power (DESIGN §15.2)
  const settled = (q) => q.waitForFunction(() => !document.querySelector("[data-pending]"));
  const prop = (q) => q.locator("[data-delivered]").first();
  const ABOVE_CAP = ["vo2max", "sprint"];
  p = await page("?stop=1");
  check("Medical stop: no wish matrix and no proposal", (await p.locator('[aria-label="Your wish"]').count()) === 0 && (await p.locator('[aria-label="Today\'s proposal"]').count()) === 0);
  p = await page("?scene=today-wish-capped");
  const capW = 280;
  const lockedCells = p.locator('.wish .matrix [data-cell^="vo2max."], .wish .matrix [data-cell^="sprint."], .wish .matrix [data-cell$=".hills"], .wish .matrix [data-cell$=".open"], .wish .matrix [data-cell$=".race"]');
  const nLocked = await lockedCells.count();
  let allLocked = nLocked > 0;
  for (let i = 0; i < nLocked; i++) allLocked &&= await lockedCells.nth(i).isDisabled();
  check("Cap: every VO2max / sprint / race-sim cell and every non-ERG format (hills in SIM, open ride) is disabled", allLocked, `${nLocked} cells`);
  const open = p.locator(".wish .matrix button.cell:not([disabled])");
  const bad = [];
  const delivered = async (label) => {
    const [i, w, free] = [await prop(p).getAttribute("data-delivered"), Number(await prop(p).getAttribute("data-max-w")), Number(await prop(p).getAttribute("data-free"))];
    if (ABOVE_CAP.includes(i) || w > capW + 0.5 || free > 0) bad.push(`${label} → ${i}, max ${w} W, ${free} free blocks`);
  };
  for (let i = 0; i < (await open.count()); i++) {
    const label = await open.nth(i).getAttribute("data-cell");
    await open.nth(i).click();
    await settled(p);
    await delivered(label);
    const own = p.getByRole("button", { name: /^Ride (my wish|the full)/ });
    if (await own.count()) {
      await own.click();
      await settled(p);
      await delivered(`${label} ridden anyway`);
    }
  }
  check("Cap: no wish, override or format delivers a target above the cap or a block the trainer can't hold", bad.length === 0, bad.join("; "));
  await p.getByRole("button", { name: "Let the coach pick" }).click();
  await settled(p);
  for (let i = 0; i < 8; i++) await p.keyboard.press("+");
  await settled(p);
  check("Cap: Harder / Intensity + from the saddle stops at the cap", (await prop(p).getAttribute("data-delivered")) === "threshold");
  p = await page("?scene=workouts-start-capped");
  check("Cap: a workout above the cap is refused at start (every path, one guard)", (await p.locator('[aria-label="Ride blocked by the health cap"]').count()) === 1);
  p = await page("?scene=ride-cap-exceeded");
  check("Cap: power above the cap in a ride is flagged and logged", (await p.locator(".cap-chip.over").textContent())?.includes("above cap · logged") ?? false);

  // 5. the saddle path reaches every coherent cell, format included, with the remote's actions only
  p = await page("?scene=today-wish-granted");
  await p.keyboard.press("s");
  const seen = new Set();
  for (let i = 0; i < 40; i++) {
    seen.add(await p.locator(".remote-wish .cell.cursor").getAttribute("data-cell"));
    await p.keyboard.press(" ");
  }
  check("Remote: Pause alone steps through all 19 coherent cells (two-button remote)", seen.size === 19, `${seen.size}`);
  const at = () => p.locator(".remote-wish .cell.cursor").getAttribute("data-cell");
  for (let i = 0; i < 5 && !(await at())?.endsWith(".hills"); i++) await p.keyboard.press("s");
  for (let i = 0; i < 6; i++) await p.keyboard.press("-");
  const cursor = await at();
  await p.keyboard.press("Enter");
  await settled(p);
  check("Remote: Skip changes format, − reaches Easy, Coach uses the wish", cursor === "easy.hills" && (await p.locator(".remote-wish").count()) === 0 && (await p.locator(`.wish .cell.wished[data-cell="${cursor}"]`).count()) === 1, cursor ?? "");
  p = await page("?scene=today-wish-adapted");
  check("Adapted wish is drawn '≈ wish', never '✓ wish', with a one-tap fix", (await p.locator('.wish .cell.wished').textContent()) === "≈ wish" && (await p.getByRole("button", { name: /^Ride the full/ }).count()) === 1);

  // 6. readiness deviations: explained with a pack rule; the rider may override (and only then)
  p = await page("?scene=today-wish-deviates");
  const line = p.locator(".outcome-line");
  check("Deviation: explained with a pack rule and its cut-off grade", (await line.getAttribute("data-relation")) === "deferred" && (await p.locator(".saddle-proposal .why").count()) === 1 && (await prop(p).getAttribute("data-delivered")) === "tempo");
  await p.getByRole("button", { name: "Ride my wish" }).click();
  await settled(p);
  check("…'ride my wish' gives the rider's wish, logged", (await prop(p).getAttribute("data-delivered")) === "vo2max" && (await line.getAttribute("data-relation")) === "anyway");
  p = await page("?scene=today-wish-deviates");
  await p.getByRole("button", { name: /Easier/ }).click();
  await settled(p);
  check("Easier = wish − from what is proposed (one outcome table)", (await prop(p).getAttribute("data-delivered")) === "easy");
} finally {
  await browser.close();
  await server.close();
}
for (const r of results) console.log(`${r.ok ? "✓" : "✗"} ${r.name}${r.detail ? ` (${r.detail})` : ""}`);
if (errors.length) console.error(`page errors:\n${errors.join("\n")}`);
const failed = results.filter((r) => !r.ok).length;
console.log(`${results.length - failed}/${results.length} passed`);
if (failed || errors.length) process.exit(1);
