// The design harness (#457, DESIGN §18.7): every fixture of e2e/design/scenarios.ts at 1024×768 and 1440×900, in
// both themes, audited by e2e/design/layout-audit.js and axe. Chromium from nixpkgs (`playwright-driver.browsers`,
// the devShell's PLAYWRIGHT_BROWSERS_PATH) stands in for the app's WKWebView; the WebdriverIO specs (`just e2e`)
// keep testing the real app. `CRANK_DESIGN_BASELINE=update` rewrites e2e/design/baseline.json (one worker). The visual
// baselines (#459) are Linux only and live under e2e/design/screenshots/<platform>/; `--update-snapshots=all` rewrites them.
import { defineConfig } from "@playwright/test";

const PORT = 1431;
const update = process.env.CRANK_DESIGN_BASELINE === "update";
const VIEWPORTS = [
  { width: 1024, height: 768 },
  { width: 1440, height: 900 },
];
export default defineConfig({
  testDir: "e2e/design",
  // Not `*.spec.ts`: Vitest would collect it.
  testMatch: "*.pw.ts",
  outputDir: "e2e-out/design",
  // A missing or different PNG fails; only `--update-snapshots=all` (the flake's `design-screenshots`) writes them.
  updateSnapshots: "none",
  // One PNG per fixture × project; the name is the fixture's (`toHaveScreenshot("<scenario>.png")`).
  snapshotPathTemplate:
    "{testDir}/screenshots/{platform}/{projectName}/{arg}{ext}",
  expect: {
    // Playwright's defaults: per-pixel colour threshold 0.2 and no `maxDiffPixels`, so one pixel past it fails. Motion stops at its
    // end state and the caret is hidden (the fixture's CSS reset does the same for anything Playwright misses).
    toHaveScreenshot: { animations: "disabled", caret: "hide", scale: "css" },
  },
  forbidOnly: true,
  workers: update ? 1 : undefined,
  // The list reporter drops annotations: e2e/design/warnings.ts prints the baseline's warnings after the run.
  reporter: [["list"], ["./e2e/design/warnings.ts"]],
  use: {
    baseURL: `http://127.0.0.1:${PORT}`,
    // The start screen hands off at once; fixtures render the same in every zone.
    reducedMotion: "reduce",
    timezoneId: "UTC",
    locale: "en-US",
  },
  projects: VIEWPORTS.flatMap((viewport) =>
    (["dark", "light"] as const).map((colorScheme) => ({
      name: `${viewport.width}x${viewport.height}-${colorScheme}`,
      use: { viewport, colorScheme },
    })),
  ),
  webServer: {
    command: `vite --config e2e/design/vite.config.ts --host 127.0.0.1 --port ${PORT} --strictPort`,
    url: `http://127.0.0.1:${PORT}/?scenario=today`,
    reuseExistingServer: false,
  },
});
