// The design harness (#457): opens each fixture (scenarios.ts), settles it, and counts the layout audit's and axe's
// violations per rule against the committed baseline (baseline.ts). DESIGN §18.7 names the rules.
import { join } from "node:path";
import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import {
  compare,
  orphans,
  pruneBaseline,
  readBaseline,
  updating,
  writeBaseline,
  type Counts,
} from "./baseline";
import { SCENARIOS, type Scenario } from "./scenarios";
import WarningsReporter, { WARNING } from "./warnings";

const AUDIT = join(import.meta.dirname, "layout-audit.js");
/** Direction A's allowances (§18.4): the rail's items are 40 px; every other control is `--control-h`. */
const AUDIT_OPTS = {
  controls: [{ selector: ".rail button", token: "--control-h-m" }],
};
/** WCAG 2.2 A and AA (rule 7). */
const AXE_TAGS = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"];

declare global {
  interface Window {
    __fixture: { settled(): Promise<void> };
    __layoutAudit(opts: object): {
      counts: Counts;
      worst: Record<string, string[]>;
    };
  }
}

async function open(page: Page, s: Scenario) {
  await page.goto(`/?scenario=${s.name}`);
  for (const name of s.clicks ?? [])
    await page.getByRole("button", { name }).first().click();
  await expect(page.locator(s.ready).first()).toBeVisible();
  await page.evaluate(() => window.__fixture.settled());
}

/** The audit's counts, then axe's as `axe.<rule>` (nodes per rule). */
async function measure(page: Page) {
  await page.addScriptTag({ path: AUDIT });
  const audit = await page.evaluate((o) => window.__layoutAudit(o), AUDIT_OPTS);
  const axe = await new AxeBuilder({ page }).withTags(AXE_TAGS).analyze();
  const counts: Counts = { ...audit.counts };
  const worst: Record<string, string[]> = { ...audit.worst };
  for (const v of axe.violations) {
    counts[`axe.${v.id}`] = v.nodes.length;
    worst[`axe.${v.id}`] = v.nodes.slice(0, 3).map((n) => n.target.join(" "));
  }
  return { counts, worst };
}

const baseline = readBaseline();
const keyOf = (s: Scenario, project: string) => `${s.name}@${project}`;

for (const s of SCENARIOS)
  test(s.name, async ({ page }, info) => {
    await open(page, s);
    const { counts, worst } = await measure(page);
    const key = keyOf(s, info.project.name);
    if (updating) return writeBaseline(key, counts);
    const { failures, warnings } = compare(baseline[key] ?? {}, counts);
    for (const w of warnings)
      info.annotations.push({ type: WARNING, description: w });
    const detail = failures.map(
      (f) => `${f}\n    ${(worst[f.split(":")[0]] ?? []).join("\n    ")}`,
    );
    expect(detail, `${key} regressed`).toEqual([]);
  });

// The gate catches what it is for. No screen passes `button.height` today (rail 56 px, ride 65 px, section nav 40 px),
// so a 48 px button shows two ways: over the baseline of a rule that is still a warning (`button.height`), and, with
// padding off the `--space-*` scale, a first violation of a rule that passes on every screen (`button.padding`).
test("an injected 48 px button fails the gate", async ({ page }, info) => {
  test.skip(updating, "measures against the committed baseline");
  const s = SCENARIOS[0];
  const base = baseline[keyOf(s, info.project.name)] ?? {};
  expect(base["button.padding"], "button.padding passes today").toBeUndefined();
  await open(page, s);
  await page.evaluate(() => {
    const b = document.createElement("button");
    b.textContent = "Injected";
    b.style.height = "48px";
    b.style.padding = "0 10px";
    document.querySelector("main")?.append(b);
  });
  const { failures } = compare(base, (await measure(page)).counts);
  const failed = failures.map((f) => f.split(":")[0]);
  expect(failed).toContain("button.height");
  expect(failed).toContain("button.padding");
});

// Axe's half of the gate: an image with no text alternative is a first `axe.image-alt` violation (WCAG 1.1.1).
test("an injected unlabelled image fails the gate", async ({ page }, info) => {
  test.skip(updating, "measures against the committed baseline");
  const s = SCENARIOS[0];
  const base = baseline[keyOf(s, info.project.name)] ?? {};
  expect(base["axe.image-alt"], "axe.image-alt passes today").toBeUndefined();
  await open(page, s);
  await page.evaluate(() => {
    const img = document.createElement("img");
    img.src = "data:image/gif;base64,R0lGODlhAQABAAAAACw=";
    document.querySelector("main")?.append(img);
  });
  const { failures } = compare(base, (await measure(page)).counts);
  expect(failures).toContain("axe.image-alt: 1 > baseline 0");
});

// Every baseline key is a fixture × project of this run; update mode drops the rest (a renamed or deleted scenario).
test("the baseline has no orphan keys", () => {
  const info = test.info();
  const expected = new Set(
    info.config.projects.flatMap((p) => SCENARIOS.map((s) => keyOf(s, p.name))),
  );
  if (updating) return void pruneBaseline(expected);
  expect(orphans(readBaseline(), expected)).toEqual([]);
});

// A count under its baseline is a printed line (the reporter, as Playwright runs it), so the slack is lowered rather
// than left for a regression.
test("an improved count prints a warning line", () => {
  const { failures, warnings } = compare(
    { "button.height": 3, "axe.color-contrast": 1 },
    { "button.height": 2 },
  );
  expect(failures).toEqual([]);
  const printed: string[] = [];
  const reporter = new WarningsReporter({ print: (l) => printed.push(l) });
  reporter.onTestEnd(
    { titlePath: () => ["", "1024x768-dark", "design.pw.ts", "ride-paused"] },
    {
      annotations: warnings.map((description) => ({
        type: WARNING,
        description,
      })),
    },
  );
  reporter.onEnd();
  expect(printed).toEqual([
    "design baseline warnings (2):",
    "design warning: [1024x768-dark] ride-paused: axe.color-contrast: 0 < baseline 1: lower it",
    "design warning: [1024x768-dark] ride-paused: button.height: 2 < baseline 3: lower it",
  ]);
});
