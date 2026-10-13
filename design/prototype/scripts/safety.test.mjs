// pnpm test:safety — the medical stop is a global invariant (DESIGN §5.8). Drives the prototype with
// Playwright and fails unless: every start path is blocked under a stop (only the ticked free ride
// starts), the daily re-check asks every red flag with no default, and the in-ride symptom action
// stops the workout and sets the stop.
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
} finally {
  await browser.close();
  await server.close();
}
for (const r of results) console.log(`${r.ok ? "✓" : "✗"} ${r.name}${r.detail ? ` (${r.detail})` : ""}`);
if (errors.length) console.error(`page errors:\n${errors.join("\n")}`);
const failed = results.filter((r) => !r.ok).length;
console.log(`${results.length - failed}/${results.length} passed`);
if (failed || errors.length) process.exit(1);
