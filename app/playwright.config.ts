// The design harness (#457, DESIGN §18.7): every fixture of e2e/design/scenarios.ts at 1024×768 and 1440×900, in
// both themes, audited by e2e/design/layout-audit.js and axe. Chromium from nixpkgs (`playwright-driver.browsers`,
// the devShell's PLAYWRIGHT_BROWSERS_PATH) stands in for the app's WKWebView; the WebdriverIO specs (`just e2e`)
// keep testing the real app. `CRANK_DESIGN_BASELINE=update` rewrites e2e/design/baseline.json (one worker).
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
  forbidOnly: true,
  workers: update ? 1 : undefined,
  reporter: [["list"]],
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
