// `pnpm shots`: start the Vite dev server, drive the app with Playwright and write PNGs to
// shots/ in dark and light mode. The ride is the real 45′ demo at 60× (no fakes); waits are
// on DOM state (data-block / data-phase on the ride root), not on fixed sleeps.
import { createServer } from "vite";
import { chromium } from "playwright";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const out = new URL("../shots/", import.meta.url);
await mkdir(out, { recursive: true });

const server = await createServer({ root, server: { port: 0, strictPort: false }, logLevel: "warn" });
await server.listen();
const addr = server.httpServer?.address();
const base = `http://localhost:${typeof addr === "object" && addr ? addr.port : 5173}/`;
const browser = await chromium.launch();
const errors = [];

const shot = async (page, name) => {
  await page.waitForTimeout(400);
  await page.screenshot({ path: fileURLToPath(new URL(`${name}.png`, out)) });
  console.log(`  ✓ ${name}.png`);
};
const setSpeed = (page, s) => page.evaluate((v) => window.__neo.ride.setSpeed(v), s);
const waitBlock = (page, i, phase) =>
  page.waitForSelector(`.ride[data-block="${i}"]${phase ? `[data-phase="${phase}"]` : ""}`, { timeout: 120_000 });

async function newPage(mode, hash = "", query = "") {
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1 });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => errors.push(`${mode}: ${e.message}`));
  page.on("console", (m) => m.type() === "error" && errors.push(`${mode}: ${m.text()}`));
  await page.goto(`${base}?mode=${mode}${query}${hash}`);
  return page;
}

for (const mode of ["dark", "light"]) {
  console.log(`[${mode}]`);

  // --- ride: one real ride, captured at the right moments ---
  const page = await newPage(mode, "", "&speed=60");
  // Recovery page during the first hr_recovery block (block 13 = index 12); auto page switch
  await waitBlock(page, 12, "recovery");
  await setSpeed(page, 10);
  await page.waitForTimeout(5000);
  await shot(page, `ride-recovery-${mode}`);
  // Interval page mid-interval: first 4′ threshold block (index 35)
  await setSpeed(page, 60);
  await waitBlock(page, 35, "work");
  await setSpeed(page, 10);
  await page.waitForTimeout(6000);
  await shot(page, `ride-interval-${mode}`);
  // Nerd page (turn auto page switch off first so the phase doesn't flip it back)
  await page.getByLabel("auto").uncheck();
  await page.keyboard.press("ArrowLeft");
  await page.waitForTimeout(2500);
  await shot(page, `ride-nerd-${mode}`);
  // Layout edit mode with a widget selected
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("e");
  await page.locator(".cell[data-kind=number_sparkline] .overlay").click();
  await shot(page, `layout-edit-${mode}`);
  await page.keyboard.press("Escape");
  // Review (mid/late ride, full page)
  await setSpeed(page, 60);
  await page.waitForTimeout(3000);
  await page.getByRole("button", { name: "Exit" }).click();
  await page.setViewportSize({ width: 1440, height: 1900 }); // the review scrolls inside <main>
  await page.waitForTimeout(2500);
  await page.screenshot({ path: fileURLToPath(new URL(`review-${mode}.png`, out)) });
  console.log(`  ✓ review-${mode}.png`);
  await page.context().close();

  // --- editor: nested repeat selected, then AI rewrite diff ---
  const ed = await newPage(mode, "#/editor", "&autoplay=0");
  await ed.getByText("5×", { exact: true }).click();
  await shot(ed, `editor-nested-${mode}`);
  await ed.getByRole("button", { name: "Harder" }).click();
  await ed.waitForSelector(".proposal", { timeout: 5000 });
  await shot(ed, `ai-diff-${mode}`);
  await ed.context().close();

  // --- check-in → proposal ---
  const ck = await newPage(mode, "#/checkin", "&autoplay=0");
  const radios = ck.getByRole("radio");
  await radios.nth(3).click(); // sleep 4
  await radios.nth(6).click(); // legs 2
  await radios.nth(11).click(); // stress 2
  await radios.nth(18).click(); // motivation 4
  await ck.getByRole("button", { name: "45′" }).click();
  await ck.getByRole("button", { name: "Get my workout" }).click();
  await ck.waitForSelector(".proposal", { timeout: 5000 });
  await shot(ck, `checkin-${mode}`);
  await ck.context().close();
}

await browser.close();
await server.close();
if (errors.length) {
  console.error(`console/page errors:\n${errors.join("\n")}`);
  process.exit(1);
}
console.log(`done → ${fileURLToPath(out)}`);
