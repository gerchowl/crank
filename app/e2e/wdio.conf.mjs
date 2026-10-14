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
  specs: [fileURLToPath(new URL("./specs/*.e2e.mjs", import.meta.url))],
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
