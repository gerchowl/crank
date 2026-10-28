// Harness smoke spec (#35): launch, element query, IPC round trip (core_ready, workouts_list) with a DOM change, screenshots.
// The shell's views are placeholders, so this exercises an event-driven DOM change instead of navigation;
// navigation and the screenshot matrix arrive with the UI waves (see the deferred #35 follow-ups).
import { readFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { browser, $, expect } from "@wdio/globals";

const outDir = process.env.CRANK_E2E_OUT ?? "e2e-out";

describe("crank shell", () => {
  it("launches and renders the shell", async () => {
    await expect(browser).toHaveTitle("crank");
    await expect($("nav[aria-label='Places']")).toBeDisplayed();
    await expect($("h1")).toHaveText("Today");
  });

  // The app runs on the device simulator with a scratch data dir (`CRANK_SIM`, `CRANK_DATA_DIR`, set by
  // scripts/e2e.sh), so these calls need no hardware (#34).
  it("round-trips core_ready and workouts_list and updates the DOM", async () => {
    const result = await browser.executeAsync((done) => {
      const invoke = window.__TAURI_INTERNALS__.invoke;
      Promise.all([invoke("core_ready"), invoke("workouts_list")])
        .then(([info, workouts]) => {
          const node = document.createElement("output");
          node.id = "e2e-probe";
          node.textContent = `sim=${info.sim} workouts=${workouts.length}`;
          document.body.append(node);
          done({ sim: info.sim, workouts: workouts.length });
        })
        .catch((error) => done(`error: ${JSON.stringify(error)}`));
    });
    expect(result.sim).toBe(true);
    expect(result.workouts).toBeGreaterThan(0);
    await expect($("#e2e-probe")).toHaveText(
      `sim=true workouts=${result.workouts}`,
    );
  });

  it("writes byte-identical screenshots for an unchanged page", async () => {
    mkdirSync(outDir, { recursive: true });
    const first = join(outDir, "shell-1.png");
    const second = join(outDir, "shell-2.png");
    await browser.saveScreenshot(first);
    await browser.saveScreenshot(second);
    const a = readFileSync(first);
    const b = readFileSync(second);
    expect(a.length).toBeGreaterThan(1000);
    expect(Buffer.compare(a, b)).toBe(0);
  });
});
