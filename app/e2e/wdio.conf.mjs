// WebdriverIO configuration for the macOS e2e harness (ADR-0001 §8.3, #35).
// Drives the Nix-built `crank-desktop-e2e` through the embedded WebDriver server
// (tauri-plugin-wdio-webdriver); run via `just e2e`, never from the production build.
import { fileURLToPath } from "node:url";

const application = process.env.CRANK_E2E_APP;
if (!application) {
  throw new Error(
    "CRANK_E2E_APP must point at the crank-desktop-e2e binary (use `just e2e`)",
  );
}

export const config = {
  runner: "local",
  // In this order, because the specs share one app process (the embedded WebDriver keeps it across sessions) and its state.
  // The smoke spec needs the fresh shell: its screenshots are compared across launches, and a ride would change Today.
  // The MVP walk (#667) pairs and rides, then quits the app; the service starts it again on the same data dir for the
  // relaunch spec (step 9). The stream spec leaves its ride loaded, so it goes last.
  specs: [
    "smoke.e2e.mjs",
    "mvp.e2e.mjs",
    "mvp-relaunch.e2e.mjs",
    "stream.e2e.mjs",
  ].map((spec) => fileURLToPath(new URL(`./specs/${spec}`, import.meta.url))),
  maxInstances: 1,
  framework: "mocha",
  mochaOpts: { timeout: 60_000 },
  reporters: ["spec"],
  logLevel: "warn",
  services: [["@wdio/tauri-service", { driverProvider: "embedded" }]],
  capabilities: [{ browserName: "tauri", "tauri:options": { application } }],
  // The shell is single-window. The service otherwise asks `tauri-plugin-wdio` (not built into the
  // app, ADR-0001 §8.3) for window states before every query and waits 5 s per command; an explicit
  // WebDriver window switch turns that auto-focus off.
  before: async (_capabilities, _specs, browser) => {
    const [handle] = await browser.getWindowHandles();
    await browser.switchToWindow(handle);
  },
};
