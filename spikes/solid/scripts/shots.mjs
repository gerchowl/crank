// pnpm shots — builds, serves with `vite preview`, drives the app with Playwright and writes
// screenshots to shots/ in dark and light mode.
import { spawn, execSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { chromium } from "playwright";

const ROOT = new URL("..", import.meta.url).pathname;
const OUT = `${ROOT}shots`;
const PORT = 4179;
const BASE = `http://localhost:${PORT}/`;
mkdirSync(OUT, { recursive: true });

if (!process.env.SKIP_BUILD) execSync("pnpm build", { cwd: ROOT, stdio: "inherit" });
const server = spawn("pnpm", ["exec", "vite", "preview", "--port", String(PORT), "--strictPort"], { cwd: ROOT, stdio: "ignore" });
const stop = () => server.kill("SIGTERM");
process.on("exit", stop);

async function waitForServer() {
  for (let i = 0; i < 100; i++) {
    try {
      const r = await fetch(BASE);
      if (r.ok) return;
    } catch {
      /* not up yet */
    }
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error("preview server did not start");
}

async function run(mode) {
  const browser = await chromium.launch();
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1 });
  await ctx.addInitScript((m) => {
    if (!sessionStorage.getItem("shots-init")) {
      localStorage.clear();
      localStorage.setItem("neo-trainer.theme.v1", JSON.stringify({ mode: m, preset: "ocean", classicZones: false }));
      sessionStorage.setItem("shots-init", "1");
    }
  }, mode);
  const page = await ctx.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  const shot = async (name, opts = {}) => {
    await page.screenshot({ path: `${OUT}/${name}-${mode}.png`, ...opts });
    console.log(`  ✓ ${name}-${mode}.png`);
  };
  const go = (hash) => page.evaluate((h) => (location.hash = h), hash);
  const key = (k) => page.keyboard.press(k);

  console.log(`[${mode}]`);
  await page.goto(`${BASE}#/ride?speed=60`);

  // 1. Interval page mid-interval: race to the first work block, then drop to 1×
  await page.waitForSelector('.ride[data-phase="work"][data-page="interval"]', { timeout: 60000 });
  await key("1");
  await page.waitForTimeout(6000);
  await shot("ride-interval");

  // 2. Recovery page during an hr_recovery block (auto page switch by phase)
  await key("3");
  await page.waitForSelector('.ride[data-page="recovery"] .hrrec[data-active="true"]', { timeout: 90000 });
  await key("2");
  await page.waitForTimeout(4000);
  await shot("ride-recovery");

  // 3. Nerd page
  await page.click('.page-dots button:has-text("Nerd")');
  await page.click('.topbar label.toggle input'); // auto page switch off while we look around
  await page.waitForTimeout(3500);
  await shot("ride-nerd");

  // 4. Layout edit mode with a widget selected
  await page.click('.page-dots button:has-text("Interval")');
  await key("e");
  await page.click('[data-widget="power"] .edit-overlay', { position: { x: 120, y: 60 } });
  await page.waitForTimeout(800);
  await shot("layout-edit");
  await key("Escape");

  // 5. Editor with a nested repeat selected
  await go("/editor");
  await page.waitForSelector(".ol-row.repeat");
  await page.locator(".ol-row.repeat").nth(1).click({ position: { x: 132, y: 12 } });
  await page.waitForTimeout(400);
  await shot("editor-nested", { fullPage: true });

  // 6. AI rewrite diff
  await page.click("button.ai:has-text('Harder')");
  await page.waitForSelector(".diff-banner:has-text('Accept')", { timeout: 10000 });
  await page.waitForTimeout(300);
  await shot("editor-ai-diff", { fullPage: true });
  await page.click(".diff-banner button:has-text('Reject')");

  // 7. Check-in with proposal
  await go("/checkin");
  await page.waitForSelector(".ci-dot");
  await page.click('[aria-label="Sleep"] .ci-dot:has-text("4")');
  await page.click('[aria-label="Legs"] .ci-dot:has-text("2")');
  await page.click('.ci-chip:has-text("45")');
  await page.click("button[type=submit]");
  await page.waitForSelector(".proposal-name", { timeout: 10000 });
  await page.waitForTimeout(400);
  await shot("checkin", { fullPage: true });

  // 8. Review after finishing the ride at 60×
  await go("/ride");
  await page.waitForSelector(".ride");
  await key("3");
  await page.waitForSelector('.ride[data-state="finished"]', { timeout: 120000 });
  await go("/review");
  await page.waitForSelector(".decision", { timeout: 10000 }).catch(() => {});
  await page.waitForTimeout(2500);
  await shot("review", { fullPage: true });

  if (errors.length) console.log("  console errors:\n   " + errors.join("\n   "));
  await browser.close();
  return errors.length;
}

try {
  await waitForServer();
  let errs = 0;
  for (const mode of ["dark", "light"]) errs += await run(mode);
  console.log(errs ? `done with ${errs} console errors` : "done, no console errors");
} finally {
  stop();
}
