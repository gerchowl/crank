// The baseline's warnings, printed (#457). A fixture at or under its baseline passes but records each rule still above
// zero, and each rule now under its baseline, as a `design-warning` annotation. The list reporter never prints
// annotations, so this reporter does, after the run, locally and in the CI log: a count that improved is a line to act
// on ("lower it"), not slack a later regression can use unseen. Skipped tests are counted by reason, so a platform
// that skips the visual baselines (#459) says so in its log.
import type { Reporter, TestCase, TestResult } from "@playwright/test/reporter";

/** The annotation type the spec records a baseline warning under. */
export const WARNING = "design-warning";

/** One printed line per warning of a finished test. */
export const warningLines = (
  test: Pick<TestCase, "titlePath">,
  result: Pick<TestResult, "annotations">,
): string[] =>
  result.annotations
    .filter((a) => a.type === WARNING)
    .map((a) => {
      // ["", project, file, title]: name the project and the fixture.
      const path = test.titlePath().filter(Boolean);
      return `design warning: [${path[0]}] ${path.at(-1)}: ${a.description}`;
    });

export default class WarningsReporter implements Reporter {
  private lines: string[] = [];
  private skipped = new Map<string, number>();
  private readonly print: (line: string) => void;
  /** Playwright passes the reporter's options (none in playwright.config.ts); the test passes `print`. */
  constructor(options: { print?: (line: string) => void } = {}) {
    this.print = options.print ?? ((l) => process.stdout.write(`${l}\n`));
  }
  onTestEnd(
    test: Pick<TestCase, "titlePath">,
    result: Pick<TestResult, "annotations">,
  ): void {
    this.lines.push(...warningLines(test, result));
    for (const a of result.annotations)
      if (a.type === "skip" && a.description)
        this.skipped.set(
          a.description,
          (this.skipped.get(a.description) ?? 0) + 1,
        );
  }
  onEnd(): void {
    for (const [reason, n] of [...this.skipped].sort())
      this.print(`design skipped (${n}): ${reason}`);
    if (!this.lines.length) return;
    this.print(`design baseline warnings (${this.lines.length}):`);
    // Sorted: workers finish in any order, the log should not.
    for (const l of this.lines.sort()) this.print(l);
  }
}
