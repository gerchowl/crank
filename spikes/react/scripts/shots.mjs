// pnpm shots — starts a Vite dev server, drives the app with Playwright and writes PNGs to shots/
// (dark and light). The demo ride runs at 60× so the whole tour takes ~2 minutes.
import { mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";

const root = fileURLToPath(new URL("..", import.meta.url));
const out = fileURLToPath(new URL("../shots/", import.meta.url));
mkdirSync(out, { recursive: true });

const server = await createServer({ root, logLevel: "warn", server: { port: 5199, strictPort: false } });
await server.listen();
const base = server.resolvedUrls?.local[0] ?? "http://localhost:5199/";
const browser = await chromium.launch();
const errors = [];

const clockS = async (page) => {
  const t = await page.locator(".clock b").innerText();
  const parts = t.split(":").map(Number);
  return parts.reduce((a, v) => a * 60 + v, 0);
};
async function waitRide(page, minS, phase) {
  for (let i = 0; i < 600; i++) {
    const ph = await page.locator(".topbar").getAttribute("data-phase");
    if ((await clockS(page)) >= minS && ph === phase) return;
    await page.waitForTimeout(100);
  }
  throw new Error(`timeout waiting for ${phase} after ${minS}s`);
}

async function tour(mode) {
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1 });
  // fresh layouts/theme for every run
  await ctx.addInitScript(() => localStorage.clear());
  const shot = async (page, name, full = false) => {
    await page.waitForTimeout(300);
    await page.screenshot({ path: `${out}${name}-${mode}.png`, fullPage: full });
    console.log(`  ${name}-${mode}.png`);
  };
  const open = async (hash, q = "") => {
    const page = await ctx.newPage();
    page.on("pageerror", (e) => errors.push(`${mode} ${hash}: ${e.message}`));
    page.on("console", (m) => { if (m.type() === "error") errors.push(`${mode} ${hash}: ${m.text()}`); });
    await page.goto(`${base}?mode=${mode}&${q}#/${hash}`);
    return page;
  };

  // ---- ride: recovery → interval → nerd → layout edit → review ----
  const ride = await open("ride", "speed=60");
  await waitRide(ride, 15 * 60 + 30, "recovery");
  await ride.waitForTimeout(1200);
  await shot(ride, "ride-recovery");
  await waitRide(ride, 33 * 60 + 50, "work");
  await ride.waitForTimeout(800);
  await shot(ride, "ride-interval");
  await ride.keyboard.press("2"); // back to 10× so the remaining shots are stable
  await ride.keyboard.press("ArrowLeft"); // interval → nerd (wraps)
  await ride.waitForTimeout(2600);
  await shot(ride, "ride-nerd");
  await ride.keyboard.press("ArrowRight");
  await ride.keyboard.press("e");
  await ride.waitForTimeout(400);
  await ride.locator(".cell").nth(2).locator(".edit-overlay").click({ position: { x: 40, y: 60 } });
  await shot(ride, "layout-edit");
  await ride.keyboard.press("e");
  await ride.getByRole("button", { name: "Exit" }).click();
  await ride.waitForTimeout(2500);
  await shot(ride, "review", true);
  await ride.close();

  // ---- editor: nested repeat selected, then AI rewrite diff ----
  const ed = await open("editor", "autostart=0");
  await ed.waitForTimeout(600);
  await ed.locator(".orow").nth(3).click({ position: { x: 8, y: 10 } });
  await shot(ed, "editor-nested-selected");
  await ed.getByRole("button", { name: "Harder" }).click();
  await ed.getByRole("button", { name: "✓ Accept" }).waitFor();
  await shot(ed, "editor-ai-diff");
  await ed.close();

  // ---- check-in → proposal ----
  const ci = await open("checkin", "autostart=0");
  await ci.locator('input[name="sleep"][value="2"]').check({ force: true });
  await ci.locator('input[name="legs"][value="2"]').check({ force: true });
  await ci.locator('input[name="motivation"][value="4"]').check({ force: true });
  await ci.locator('input[name="minutes"][value="45"]').check({ force: true });
  await ci.locator('input[name="note"]').fill("legs heavy after yesterday");
  await ci.getByRole("button", { name: /Get today/ }).click();
  await ci.getByRole("button", { name: /Accept & ride/ }).waitFor();
  await shot(ci, "checkin");
  await ci.close();
  await ctx.close();
}

try {
  for (const mode of ["dark", "light"]) {
    console.log(`${mode}:`);
    await tour(mode);
  }
} finally {
  await browser.close();
  await server.close();
}
if (errors.length) {
  console.error("page errors:\n" + errors.join("\n"));
  process.exit(1);
}
