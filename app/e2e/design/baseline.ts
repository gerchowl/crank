// The warn baseline (#457): per fixture and project, the violation count of each audit rule and axe rule as of the
// commit. A rule may not exceed its baseline count; a rule absent from the baseline passes today and fails on its
// first violation. Rules at a count above zero are reported as warnings until #460 ratchets them down one rule per PR
// (DESIGN §18.7 rule 15). `CRANK_DESIGN_BASELINE=update` rewrites the file from the current counts.
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
  writeFileSync(FILE, `${JSON.stringify(sorted(all), null, 2)}\n`);
}

/** Regressions (over the baseline: fail) and warnings (at or under it, but not zero). */
export function compare(base: Counts, now: Counts) {
  const failures: string[] = [];
  const warnings: string[] = [];
  for (const [rule, n] of Object.entries(now)) {
    const allowed = base[rule] ?? 0;
    if (n > allowed) failures.push(`${rule}: ${n} > baseline ${allowed}`);
    else warnings.push(`${rule}: ${n} (baseline ${allowed})`);
  }
  for (const [rule, n] of Object.entries(base))
    if ((now[rule] ?? 0) < n)
      warnings.push(`${rule}: ${now[rule] ?? 0} < baseline ${n}: lower it`);
  return { failures, warnings };
}
