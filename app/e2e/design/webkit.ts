// `just design-webkit` (#630): the layout audit's generic invariants (audit-opts.ts `INVARIANTS`) on every fixture at
// every harness size, in the system WKWebView the app ships in on macOS. The harness's Chromium (design.pw.ts) and
// Playwright's own WebKit build lay some things out unlike it: #630's strip drew its blocks into the key row in the
// app, and in neither of them. macOS only, in the logged-in GUI session (a WebView needs it), with the Xcode command
// line tools' `swiftc`; elsewhere it says it skipped. One appearance (the system's): the invariants are about boxes.
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { mkdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { VIEWPORTS } from "../../playwright.config.ts";
import { AUDIT_OPTS, INVARIANTS } from "./audit-opts.ts";
import { FROZEN, SCENARIOS } from "./scenarios.ts";

const HERE = import.meta.dirname;
const APP = join(HERE, "..", "..");
const OUT = join(APP, "e2e-out", "design-webkit");
const PORT = 1433;
const BASE = `http://127.0.0.1:${PORT}`;

/** Playwright's `clock.setFixedTime` for the WebView: `Date` with no argument and `Date.now()` read the fixed time. */
const frozenClock = (iso: string) => `(() => {
  const T = Date.parse(${JSON.stringify(iso)}), D = Date;
  class Fixed extends D { constructor(...a) { if (a.length) super(...a); else super(T); } static now() { return T; } }
  window.Date = Fixed;
})();`;

interface Audit {
  error?: string;
  rules?: Record<string, string[]>;
}

async function serve(): Promise<ChildProcess> {
  const vite = spawn(
    "pnpm",
    [
      "exec",
      "vite",
      "--config",
      "e2e/design/vite.config.ts",
      "--host",
      "127.0.0.1",
      "--port",
      String(PORT),
      "--strictPort",
    ],
    { cwd: APP, stdio: "ignore" },
  );
  for (let i = 0; i < 120; i++) {
    if (vite.exitCode !== null) break;
    try {
      if ((await fetch(`${BASE}/?scenario=today`)).ok) return vite;
    } catch {
      // Not listening yet.
    }
    await new Promise((r) => setTimeout(r, 250));
  }
  vite.kill();
  throw new Error(`the fixture server did not answer on ${BASE}`);
}

function audit(width: number, height: number, runner: string): Audit[] {
  const source = readFileSync(join(HERE, "layout-audit.js"), "utf8");
  const jobs = SCENARIOS.map((job) =>
    JSON.stringify({
      url: `${BASE}/?scenario=${job.name}`,
      init: frozenClock(FROZEN),
      job,
      audit: source,
      opts: AUDIT_OPTS,
    }),
  );
  const run = spawnSync(
    runner,
    [String(width), String(height), join(HERE, "webkit", "driver.js")],
    { input: jobs.join("\n") + "\n", encoding: "utf8", timeout: 600_000 },
  );
  if (run.status !== 0)
    throw new Error(`runner exited ${run.status}: ${run.stderr}`);
  const lines = run.stdout.trim().split("\n");
  if (lines.length !== SCENARIOS.length)
    throw new Error(`runner answered ${lines.length} of ${SCENARIOS.length}`);
  return lines.map((l) => JSON.parse(l) as Audit);
}

if (process.platform !== "darwin") {
  console.log(
    "design-webkit skipped: the system WKWebView exists on macOS only",
  );
  process.exit(0);
}
mkdirSync(OUT, { recursive: true });
const runner = join(OUT, "runner");
// The system toolchain and SDK: the devShell points SDKROOT and DEVELOPER_DIR at the Nix Apple SDK, which ships no
// Swift standard library.
const env = { ...process.env };
for (const k of ["SDKROOT", "DEVELOPER_DIR", "MACOSX_DEPLOYMENT_TARGET"])
  delete env[k];
const build = spawnSync(
  "/usr/bin/xcrun",
  ["swiftc", "-O", join(HERE, "webkit", "runner.swift"), "-o", runner],
  { stdio: "inherit", env },
);
if (build.status !== 0) process.exit(build.status ?? 1);

const vite = await serve();
let failed = 0;
try {
  for (const { width, height } of VIEWPORTS) {
    const results = audit(width, height, runner);
    results.forEach((r, i) => {
      const where = `[${width}x${height}] ${SCENARIOS[i].name}`;
      if (r.error) {
        failed++;
        console.log(`design-webkit: ${where}: ${r.error}`);
      }
      for (const rule of INVARIANTS)
        for (const m of r.rules?.[rule] ?? []) {
          failed++;
          console.log(`design-webkit: ${where}: ${rule}: ${m}`);
        }
    });
  }
} finally {
  vite.kill();
}
console.log(
  `design-webkit: ${failed} violation(s) of ${INVARIANTS.join(", ")} in ${SCENARIOS.length} fixtures x ${VIEWPORTS.length} sizes`,
);
process.exit(failed ? 1 : 0);
