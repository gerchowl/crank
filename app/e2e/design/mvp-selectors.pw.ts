// The MVP walk's selectors (#667, e2e/specs/mvp-selectors.mjs) against the design fixtures. The fixtures mount the
// components the real app renders (fixture.tsx, on the mockIPC backends), so a component change that breaks a
// selector of `just e2e` (macOS, GUI session only) fails here, in CI. This checks only that the selectors resolve to
// the right nodes. What the real core makes them show is the e2e specs' job. The fixtures' device names are the mock's
// (devicesBackend.ts); the walk uses the sim core's.
import { expect, test, type Page } from "@playwright/test";
import * as sel from "../specs/mvp-selectors.mjs";
import { FROZEN } from "./scenarios";

declare global {
  interface Window {
    __fixture: { settled(): Promise<void> };
  }
}

/** Opens a fixture as the harness does (design.pw.ts `open`), without its picks, clicks or audit. */
async function open(page: Page, scenario: string) {
  await page.clock.setFixedTime(new Date(FROZEN));
  await page.goto(`/?scenario=${scenario}`);
  await page.evaluate(() => window.__fixture.settled());
}
/** Exactly one node matches, and it is visible. */
const one = async (page: Page, selector: string) => {
  const at = page.locator(selector);
  await expect(at).toHaveCount(1);
  await expect(at).toBeVisible();
  return at;
};

test("mvp selectors: rail and Today", async ({ page }) => {
  await open(page, "today");
  await expect(await one(page, sel.heading)).toHaveText("Today");
  for (const place of ["Settings", "Workouts", "History"])
    await one(page, sel.rail(place));
});

test("mvp selectors: Settings → Devices, scan and pair", async ({ page }) => {
  await open(page, "devices-empty");
  await one(page, sel.settingsSection("Devices"));
  for (const role of ["Trainer", "Heart-rate strap"]) {
    await expect(await one(page, sel.pairedRow(role))).toContainText(
      "Not paired",
    );
    await expect(page.locator(sel.pairedLine(role))).toHaveCount(0);
  }
  await (await one(page, sel.scanButton)).click();
  await one(page, sel.pairButton("trainer", "Tacx Neo 55624"));
  await (await one(page, sel.pairButton("strap", "HRM-Dual"))).click();
  await one(page, sel.pairedMark("strap", "HRM-Dual"));
  await one(page, sel.pairedLine("Heart-rate strap"));
  // The scan button is one node in both its words.
  await expect(await one(page, sel.scanButton)).toHaveText("Scan again");
});

test("mvp selectors: both paired", async ({ page }) => {
  await open(page, "devices-paired");
  await (await one(page, sel.scanButton)).click();
  await one(page, sel.pairedMark("trainer", "Tacx Neo 55624"));
  await one(page, sel.pairedMark("strap", "HRM-Dual"));
  await one(page, sel.pairedLine("Trainer"));
  await one(page, sel.pairedLine("Heart-rate strap"));
});

test("mvp selectors: Workouts, pick and Ride this", async ({ page }) => {
  await open(page, "workouts");
  await (await one(page, sel.workoutOption("Sweet spot 3x12"))).click();
  await expect(await one(page, sel.rideThis)).toBeEnabled();
});

test("mvp selectors: the Ready sheet", async ({ page }) => {
  await open(page, "ready");
  await one(page, sel.ride);
  await one(page, sel.readySheet("ready"));
  await one(page, sel.readyDevice("trainer", "connected"));
  await one(page, sel.readyDevice("hr", "connected"));
  await expect(await one(page, sel.readyPlan)).toHaveText(
    /^ERG · starts with \d+:\d\d /,
  );
  await expect(await one(page, sel.key("Start"))).toBeEnabled();
  await one(page, sel.key("End"));
  for (const word of ["Pause", "Resume", "Skip block"])
    await expect(page.locator(sel.key(word))).toHaveCount(0);
});

test("mvp selectors: riding, the tiles, the chart, End's dialog", async ({
  page,
}) => {
  await open(page, "ride-work-step");
  for (const word of ["Pause", "Resume", "Skip block", "End"])
    await one(page, sel.key(word));
  await expect(page.locator(sel.key("Start"))).toHaveCount(0);
  await expect(await one(page, sel.stateLine)).toHaveText("Riding");
  await expect(await one(page, sel.slot("target"))).toHaveText(/^\d+$/);
  await expect(await one(page, sel.slot("plan"))).toHaveText(/^Plan \d+ W$/);
  for (const s of ["power", "cadence", "hr"])
    await expect(await one(page, sel.slot(s))).toHaveText(/^\d+$/);
  await expect(await one(page, sel.slot("block"))).toHaveText(
    /· block \d+ of \d+$/,
  );
  await one(page, sel.slot("countdown"));
  for (const series of ["power", "hr", "cadence"] as const)
    expect(await page.locator(sel.chart(series)).count()).toBeGreaterThan(0);
  await (await one(page, sel.key("End"))).click();
  await one(page, sel.confirmEnd);
});

test("mvp selectors: paused", async ({ page }) => {
  await open(page, "ride-paused");
  await one(page, sel.banner("paused"));
  await expect(await one(page, sel.stateLine)).toHaveText("Paused");
  await expect(await one(page, sel.slot("target"))).toHaveText("Paused");
  await one(page, `${sel.slot("countdown")}[data-dim]`);
  await expect(await one(page, sel.key("Pause"))).toBeDisabled();
  await expect(await one(page, sel.key("Resume"))).toBeEnabled();
});

test("mvp selectors: the summary", async ({ page }) => {
  await open(page, "review");
  await one(page, sel.summary);
  await expect(await one(page, sel.heading)).toHaveText("Sweet spot 3x12");
  await expect(await one(page, sel.outcome)).toHaveAttribute(
    "data-outcome",
    "finished",
  );
  await expect(await one(page, sel.stat("Duration"))).toHaveText(
    /^\d+:\d\d(:\d\d)?$/,
  );
  await expect(await one(page, sel.stat("Average power"))).toHaveText(
    /^\d+ W$/,
  );
  await expect(await one(page, sel.stat("Average heart rate"))).toHaveText(
    /^\d+ bpm$/,
  );
});

test("mvp selectors: History's rides", async ({ page }) => {
  await open(page, "history");
  await expect(await one(page, sel.heading)).toHaveText("History");
  expect(await page.locator(sel.historyRows).count()).toBeGreaterThan(1);
  await expect(
    await one(page, sel.reviewButton("Sweet spot 3x12")),
  ).toHaveAttribute("aria-label", /, Finished, \d+:\d\d(:\d\d)?$/);
});
