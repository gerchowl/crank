// WebdriverIO configuration for the macOS e2e harness (ADR-0001 §8.3, #35).
// Drives the Nix-built `crank-desktop-e2e` through the embedded WebDriver server
// (tauri-plugin-wdio-webdriver); run via `just e2e`, never from the production build.
import { readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";

const application = process.env.CRANK_E2E_APP;
if (!application) {
  throw new Error(
    "CRANK_E2E_APP must point at the crank-desktop-e2e binary (use `just e2e`)",
  );
}

// In this order, because the specs share one app process (the embedded WebDriver keeps it across sessions) and its state.
// The smoke spec needs the fresh shell: its screenshots are compared across launches, and a ride would change Today.
// The MVP walk (#667) pairs and rides, then quits the app; the service starts it again on the same data dir for the
// relaunch spec (step 9). The stream spec leaves its ride loaded, so it goes last.
const ORDER = [
  "smoke.e2e.mjs",
  "mvp.e2e.mjs",
  "mvp-relaunch.e2e.mjs",
  "stream.e2e.mjs",
];
// A spec that is not in ORDER would never run, and one listed but missing would be an error deep in the run: refuse
// both up front, by name.
const specsDir = fileURLToPath(new URL("./specs/", import.meta.url));
const present = readdirSync(specsDir).filter((f) => f.endsWith(".e2e.mjs"));
const unlisted = present.filter((f) => !ORDER.includes(f));
const missing = ORDER.filter((f) => !present.includes(f));
if (unlisted.length || missing.length) {
  throw new Error(
    `e2e/wdio.conf.mjs ORDER does not match specs/*.e2e.mjs: ${[
      unlisted.length && `not in ORDER: ${unlisted.join(", ")}`,
      missing.length && `in ORDER but missing: ${missing.join(", ")}`,
    ]
      .filter(Boolean)
      .join("; ")} (add each spec to ORDER where its app state allows)`,
  );
}

export const config = {
  runner: "local",
  specs: ORDER.map((spec) => specsDir + spec),
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
