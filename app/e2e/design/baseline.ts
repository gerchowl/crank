// The warn baseline (#457): per fixture and project, the violation count of each audit rule and axe rule as of the
// commit. A rule may not exceed its baseline count; a rule absent from the baseline passes today and fails on its
// first violation. Rules at a count above zero are reported as warnings until #460 ratchets them down one rule per PR
// (DESIGN §18.7 rule 15); warnings.ts prints them. `CRANK_DESIGN_BASELINE=update` rewrites the file from the current
// counts and prunes the keys of fixtures and projects that no longer exist; a check run fails on such keys.
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

/** Violations per rule; a rule with none is omitted. */
export type Counts = Record<string, number>;
type Baseline = Record<string, Counts>;

const FILE = join(import.meta.dirname, "baseline.json");
export const updating = process.env.CRANK_DESIGN_BASELINE === "update";

export const readBaseline = (): Baseline =>
  existsSync(FILE) ? (JSON.parse(readFileSync(FILE, "utf8")) as Baseline) : {};

const sorted = <T>(o: Record<string, T>): Record<string, T> =>
  Object.fromEntries(Object.entries(o).sort(([a], [b]) => a.localeCompare(b)));

/** Record `counts` under `key`. Safe only with one worker (playwright.config.ts sets it in update mode). */
export function writeBaseline(key: string, counts: Counts): void {
  const all = readBaseline();
  all[key] = sorted(counts);
  save(all);
}

const save = (all: Baseline) =>
  writeFileSync(FILE, `${JSON.stringify(sorted(all), null, 2)}\n`);

/** Keys that belong to no fixture × project of the run (a renamed or deleted scenario or project). */
export const orphans = (all: Baseline, expected: ReadonlySet<string>) =>
  Object.keys(all).filter((k) => !expected.has(k));

/** Drop the orphan keys (update mode). */
export function pruneBaseline(expected: ReadonlySet<string>): string[] {
  const all = readBaseline();
  const gone = orphans(all, expected);
  for (const k of gone) delete all[k];
  if (gone.length) save(all);
  return gone;
}

/**
 * Regressions (over the baseline: fail) and warnings: a rule still above zero at its baseline, and a rule under its
 * baseline, whose count should be lowered so a later regression cannot use the slack.
 */
export function compare(base: Counts, now: Counts) {
  const failures: string[] = [];
  const warnings: string[] = [];
  for (const rule of new Set([...Object.keys(base), ...Object.keys(now)])) {
    const n = now[rule] ?? 0;
    const allowed = base[rule] ?? 0;
    if (n > allowed) failures.push(`${rule}: ${n} > baseline ${allowed}`);
    else if (n < allowed)
      warnings.push(`${rule}: ${n} < baseline ${allowed}: lower it`);
    else warnings.push(`${rule}: ${n} (baseline ${allowed})`);
  }
  return { failures, warnings };
}
