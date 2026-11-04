// The design harness (#457): opens each fixture (scenarios.ts), settles it, and counts the layout audit's and axe's
// violations per rule against the committed baseline (baseline.ts). DESIGN §18.7 names the rules.
import { join } from "node:path";
import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import {
  compare,
  readBaseline,
  updating,
  writeBaseline,
  type Counts,
} from "./baseline";
import { SCENARIOS, type Scenario } from "./scenarios";

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
      info.annotations.push({ type: "warning", description: w });
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
