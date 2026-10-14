import { readFileSync } from "node:fs";
import { expect, test } from "vitest";

// DESIGN §16.3: content is 912 px wide at 1024 (1024 - 72 rail - 2 x padding) with no breakpoint reflow.
const css = readFileSync(new URL("./shell.css", import.meta.url), "utf8");
const m =
  /\.page \{[^}]*padding:\s*28px clamp\((\d+)px,\s*calc\(([\d.]+)vw - ([\d.]+)px\),\s*(\d+)px\)/.exec(
    css,
  );

test("page side padding yields 912 px content at 1024 and 1304 px at 1440", () => {
  expect(m).not.toBeNull();
  const [min, vw, off, max] = m!.slice(1).map(Number) as [
    number,
    number,
    number,
    number,
  ];
  const pad = (w: number) => Math.min(max, Math.max(min, (vw * w) / 100 - off));
  expect(Math.round(1024 - 72 - 2 * pad(1024))).toBe(912);
  expect(Math.round(Math.min(1368, 1440 - 72 - 2 * pad(1440)))).toBe(1304);
  expect(css).not.toMatch(/@media[^{]*max-width/); // no breakpoint reflow
});
