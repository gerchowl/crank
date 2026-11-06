// The design harness (#457): opens each fixture (scenarios.ts), settles it, and counts the layout audit's, axe's and the
// focus walk's (focus.ts, #458) violations per rule against the committed baseline (baseline.ts). DESIGN §18.7 names
// the rules. On Linux it also compares each settled fixture with its committed screenshot (#459).
import { readdirSync } from "node:fs";
import { join, relative } from "node:path";
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
import { focusWalk, inOrder, ORDER_TOLERANCE } from "./focus";
import { RIDE_SCENARIOS, SCENARIOS, type Scenario } from "./scenarios";
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

/** The fixtures' wall clock (`Date`): a ride date or a "today" renders the same on every run. Timers still run. */
const FROZEN = new Date("2026-11-02T08:00:00Z");

async function open(page: Page, s: Scenario) {
  await page.clock.setFixedTime(FROZEN);
  // No fixture draws from `Math.random` today (each is static data, scenarios.ts); one that does gets the same
  // sequence every run (mulberry32, fixed seed).
  await page.addInitScript(() => {
    let seed = 459;
    Math.random = () => {
      seed = (seed + 0x6d2b79f5) | 0;
      let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  });
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
    const audit = await measure(page);
    // After the audit: the walk moves focus, and its skip-link check moves it into `#main`.
    const focus = await focusWalk(page);
    expect(focus.stops, "the focus walk reached a control").toBeGreaterThan(0);
    const counts = { ...audit.counts, ...focus.counts };
    const worst = { ...audit.worst, ...focus.worst };
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

// Visual baselines (#459): each fixture × project against its committed PNG (playwright.config.ts names the path and
// the comparison). Linux only: the PNGs are rendered by the `app-e2e-design` check's Chromium on x86_64-linux, and
// another platform's font rasteriser and compositor draw different pixels, so a comparison there could only fail.
const SCREENSHOTS = join(import.meta.dirname, "screenshots");
const LINUX_ONLY =
  "visual baselines run on Linux only (the app-e2e-design check); this platform renders other pixels";
const linux = process.platform === "linux";
/** `--update-snapshots=all`: the run rewrites every PNG (`just design update`, via the flake's `design-screenshots`). */
const rewriting = () => test.info().config.updateSnapshots === "all";

for (const s of SCENARIOS)
  test(`${s.name} screenshot`, async ({ page }) => {
    test.skip(!linux, LINUX_ONLY);
    test.skip(
      updating,
      "the counts' update run; the PNGs come from design-screenshots",
    );
    await open(page, s);
    await expect(page).toHaveScreenshot(`${s.name}.png`);
  });

// The screenshot gate catches what it is for: a 1 px border on the rail differs from the committed PNG.
test("an injected 1 px rail border fails the screenshot", async ({ page }) => {
  test.skip(!linux, LINUX_ONLY);
  test.skip(updating || rewriting(), "compares against the committed PNG");
  const s = SCENARIOS[0];
  await open(page, s);
  await page.evaluate(() => {
    const rail = document.querySelector<HTMLElement>(".rail");
    if (!rail) throw new Error("no .rail on the first fixture");
    rail.style.border = "1px solid red";
  });
  await expect(
    expect(page).toHaveScreenshot(`${s.name}.png`, { timeout: 2000 }),
  ).rejects.toThrow(/pixels .* are different/);
});

// Every committed PNG is a Linux fixture × project of this run, on every platform: a renamed or deleted fixture
// leaves none behind (the update path renders into an empty directory). A missing one fails its own screenshot test.
test("the screenshots have no orphans", () => {
  const expected = new Set(
    test
      .info()
      .config.projects.flatMap((p) =>
        SCENARIOS.map((s) => join("linux", p.name, `${s.name}.png`)),
      ),
  );
  let found: string[] = [];
  try {
    found = readdirSync(SCREENSHOTS, { recursive: true, withFileTypes: true })
      .filter((e) => e.isFile())
      .map((e) => relative(SCREENSHOTS, join(e.parentPath, e.name)));
  } catch (e) {
    if ((e as NodeJS.ErrnoException).code !== "ENOENT") throw e;
  }
  expect(found.filter((f) => !expected.has(f))).toEqual([]);
});

// Rule 10 (no layout shift, #498): the ride's regions (DESIGN §18.2, §18.3) have the same box in every ride fixture at
// each size, so no state, banner or value moves one. Every region is there in every fixture.
const REGIONS = ["status", "traces", "banner", "metrics", "timeline", "keys"];
const regionBoxes = (page: Page) =>
  page.evaluate(() =>
    Object.fromEntries(
      [...document.querySelectorAll<HTMLElement>("[data-region]")].map((e) => {
        const r = e.getBoundingClientRect();
        return [e.dataset.region, [r.x, r.y, r.width, r.height].join(" ")];
      }),
    ),
  );
// No banner text is cut (§6.1: every line fits at 1024 and 1440) and every action in the band (End…) is wholly inside it
// and the window. The audit's `layout.clipped` sees an overflowing box; this also names the line and the action.
const bannerFit = (page: Page) =>
  page.evaluate(() => {
    const band = document.querySelector("[data-region='banner']");
    if (!band) return ["no banner band"];
    const b = band.getBoundingClientRect();
    const bad: string[] = [];
    for (const t of band.querySelectorAll<HTMLElement>(
      ".banner-text, .banner-line",
    ))
      if (t.scrollWidth > t.clientWidth + 1)
        bad.push(`cut: "${t.textContent}" ${t.scrollWidth} > ${t.clientWidth}`);
    for (const a of band.querySelectorAll("button")) {
      const r = a.getBoundingClientRect();
      if (
        r.left < b.left ||
        r.right > b.right ||
        r.top < b.top ||
        r.bottom > b.bottom ||
        r.right > innerWidth
      )
        bad.push(`action outside the band: "${a.textContent}"`);
    }
    return bad;
  });
test("the ride's region boxes are the same in every ride fixture", async ({
  page,
}) => {
  let first: Record<string, string> | null = null;
  for (const name of RIDE_SCENARIOS) {
    const s = SCENARIOS.find((x) => x.name === name);
    if (!s) throw new Error(`no fixture ${name}`);
    await open(page, s);
    const boxes = await regionBoxes(page);
    expect(Object.keys(boxes).sort(), `${name}: the regions`).toEqual(
      [...REGIONS].sort(),
    );
    first ??= boxes;
    expect(boxes, `${name} against ${RIDE_SCENARIOS[0]}`).toEqual(first);
    expect(await bannerFit(page), `${name}: banner text and actions`).toEqual(
      [],
    );
  }
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

// The rail's words sit inside their buttons with room to spare (#506, DESIGN §16.3): the layout audit's 2 px
// `layout.overflowParent` allowance let "Workouts" (600 weight, the current place) spill 2 px each side and still pass.
// The margin is `--space-1` on each side of the widest word, measured on the bundled Inter after `settled()`.
const RAIL_LABEL_MARGIN = 4;
for (const place of ["Today", "Workouts", "History", "Settings"])
  test(`the rail word ${place} has ${RAIL_LABEL_MARGIN} px inside its button at 600 weight`, async ({
    page,
  }) => {
    const s =
      SCENARIOS.find((x) => x.name === "devices-denied") ?? SCENARIOS[0];
    await open(page, s);
    const m = await page.evaluate((name) => {
      const button = [
        ...document.querySelectorAll<HTMLElement>(".rail button"),
      ].find(
        (b) => b.querySelector(".rail-label")?.textContent?.trim() === name,
      );
      const label = button?.querySelector<HTMLElement>(".rail-label");
      if (!button || !label) return null;
      // The current place is 600 weight; measure every word at that widest weight.
      const was = button.style.fontWeight;
      button.style.fontWeight = "600";
      const range = document.createRange();
      range.selectNodeContents(label);
      const t = range.getBoundingClientRect();
      const b = button.getBoundingClientRect();
      button.style.fontWeight = was;
      return {
        left: t.left - b.left,
        right: b.right - t.right,
        width: t.width,
        button: b.width,
      };
    }, place);
    expect(m, `${place} is in the rail`).not.toBeNull();
    expect(
      Math.min(m!.left, m!.right),
      `${place} (${m!.width.toFixed(2)} px) in a ${m!.button.toFixed(2)} px button`,
    ).toBeGreaterThanOrEqual(RAIL_LABEL_MARGIN);
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

// The focus walk's half (#458): a control whose ring is removed (`outline: none`, nothing in its place) is one more
// `focus.ring` violation than the baseline allows.
test("an injected control without a focus ring fails the gate", async ({
  page,
}, info) => {
  test.skip(updating, "measures against the committed baseline");
  const s = SCENARIOS[0];
  const base = baseline[keyOf(s, info.project.name)] ?? {};
  await open(page, s);
  await page.evaluate(() => {
    const b = document.createElement("button");
    b.textContent = "Injected";
    b.style.outline = "none";
    document.querySelector("main")?.append(b);
  });
  const { failures } = compare(base, (await focusWalk(page)).counts);
  expect(failures.map((f) => f.split(":")[0])).toContain("focus.ring");
});

// A control last in the DOM but drawn top left, over the rail, is reached after the rail's last item and sits above
// it in the same column: one more `focus.order` violation than the baseline allows.
test("an injected control out of DOM order fails the gate", async ({
  page,
}, info) => {
  test.skip(updating, "measures against the committed baseline");
  const s = SCENARIOS[0];
  const base = baseline[keyOf(s, info.project.name)] ?? {};
  await open(page, s);
  await page.evaluate(() => {
    const b = document.createElement("button");
    b.textContent = "Injected";
    b.style.position = "fixed";
    b.style.left = "8px";
    b.style.top = "8px";
    document.querySelector("main")?.append(b);
  });
  const { failures } = compare(base, (await focusWalk(page)).counts);
  expect(failures.map((f) => f.split(":")[0])).toContain("focus.order");
});

// The audit measures the real fonts (#504): with every font response held back, `settled()` still resolves only once
// each bundled face (theme.css: Inter 400/600, JetBrains Mono 400/700) is loaded, never on the fallback font.
test("the fixture settles on the bundled fonts", async ({ page }) => {
  let release = () => {};
  const held = new Promise<void>((r) => (release = r));
  await page.route("**/*.woff2", async (route) => {
    await held;
    await route.continue();
  });
  await page.goto(`/?scenario=${SCENARIOS[0].name}`);
  await expect(page.locator(SCENARIOS[0].ready).first()).toBeVisible();
  // The faces as they are the moment `settled()` resolves.
  const faces = page.evaluate(() =>
    window.__fixture
      .settled()
      .then(() =>
        [...document.fonts].map((f) => `${f.family} ${f.weight}: ${f.status}`),
      ),
  );
  // Text is on screen, in the fallback font, while the faces are still on the wire.
  expect(await page.evaluate(() => document.fonts.status)).toBe("loading");
  release();
  expect((await faces).sort()).toEqual([
    "Inter 400: loaded",
    "Inter 600: loaded",
    "JetBrains Mono 400: loaded",
    "JetBrains Mono 700: loaded",
  ]);
});

// The reading order (focus.ts): along a row, down to the next row from anywhere, up only into a later column.
test("the focus order follows rows, then columns", () => {
  const box = (left: number, top: number, w = 80, h = 32) => ({
    left,
    top,
    right: left + w,
    bottom: top + h,
  });
  const a = box(100, 100);
  expect(inOrder(a, box(200, 104))).toBe(true); // same row, right
  expect(inOrder(a, box(100 - ORDER_TOLERANCE, 100))).toBe(true); // same row, within the tolerance
  expect(inOrder(a, box(20, 100))).toBe(false); // same row, left
  expect(inOrder(a, box(0, 200))).toBe(true); // next row, from its start
  expect(inOrder(a, box(180, 0))).toBe(true); // a later column, from its top
  expect(inOrder(a, box(120, 0))).toBe(false); // up the same column
  expect(inOrder(a, box(0, 0))).toBe(false); // up and back left
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

// A platform that skips the visual baselines says so: one line per reason, with its count.
test("skipped tests print their reason", () => {
  const printed: string[] = [];
  const reporter = new WarningsReporter({ print: (l) => printed.push(l) });
  const t = { titlePath: () => ["", "1024x768-dark", "design.pw.ts", "today"] };
  const skip = { type: "skip", description: LINUX_ONLY };
  reporter.onTestEnd(t, { annotations: [skip] });
  reporter.onTestEnd(t, { annotations: [skip] });
  reporter.onTestEnd(t, { annotations: [{ type: "skip" }] });
  reporter.onEnd();
  expect(printed).toEqual([`design skipped (2): ${LINUX_ONLY}`]);
});
