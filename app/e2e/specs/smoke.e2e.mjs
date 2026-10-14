// Harness smoke spec (#35): launch, element query, IPC round trip with a DOM change, screenshots.
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

  it("round-trips a Rust command and updates the DOM", async () => {
    const appName = await browser.executeAsync((done) => {
      window.__TAURI_INTERNALS__
        .invoke("scaffold_info")
        .then((info) => {
          const node = document.createElement("output");
          node.id = "e2e-probe";
          node.textContent = info.appName;
          document.body.append(node);
          done(info.appName);
        })
        .catch((error) => done(`error: ${String(error)}`));
    });
    expect(appName).toBe("crank");
    await expect($("#e2e-probe")).toHaveText("crank");
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
