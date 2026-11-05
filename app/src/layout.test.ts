import { readFileSync } from "node:fs";
import { expect, test } from "vitest";

// DESIGN §16.3: content is 908 px wide at 1024 (1024 - 76 rail - 2 x padding) with no breakpoint reflow.
const css = readFileSync(new URL("./shell.css", import.meta.url), "utf8");
const theme = readFileSync(new URL("./theme.css", import.meta.url), "utf8");
const m =
  /--page-gutter:\s*clamp\((\d+)px,\s*calc\(([\d.]+)vw - ([\d.]+)px\),\s*(\d+)px\)/.exec(
    theme,
  );

test("page side padding yields 908 px content at 1024 and 1300 px at 1440", () => {
  expect(css).toMatch(/\.page \{[^}]*padding:[^;]*var\(--page-gutter\)/);
  expect(m).not.toBeNull();
  const [min, vw, off, max] = m!.slice(1).map(Number) as [
    number,
    number,
    number,
    number,
  ];
  const pad = (w: number) => Math.min(max, Math.max(min, (vw * w) / 100 - off));
  expect(Math.round(1024 - 76 - 2 * pad(1024))).toBe(908);
  expect(Math.round(Math.min(1364, 1440 - 76 - 2 * pad(1440)))).toBe(1300);
  expect(css).not.toMatch(/@media[^{]*max-width/); // no breakpoint reflow
});
