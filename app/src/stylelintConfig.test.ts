import { readFileSync } from "node:fs";
import stylelint from "stylelint";
import { beforeAll, expect, test } from "vitest";
import { FALLBACK_TOKENS } from "./typeTiers";

// stylelint.config.mjs lists the custom properties that useTypeTiers sets at runtime (no stylesheet declares
// them), so `value-no-unknown-custom-properties` accepts them. That list must be exactly the tokens it sets.
test("the stylelint runtime-token list is the type-tier tokens", () => {
  const config = readFileSync(
    new URL("../stylelint.config.mjs", import.meta.url),
    "utf8",
  );
  const list = /RUNTIME_TOKENS = \[([^\]]*)\]/.exec(config)?.[1] ?? "";
  const names = [...list.matchAll(/"(--[\w-]+)"/g)].map((m) => m[1]);
  expect(names.sort()).toEqual(Object.keys(FALLBACK_TOKENS).sort());
});

// Every literal below must fail, through longhands and shorthands alike (DESIGN §18.7 rules 1 to 3, #454).
const configFile = new URL("../stylelint.config.mjs", import.meta.url).pathname;
const lintCss = async (decl: string, file = "src/x.css") => {
  const { results } = await stylelint.lint({
    code: `.a { ${decl}; }\n`,
    codeFilename: file,
    configFile,
  });
  return results[0]!.warnings.map((w) => w.rule);
};

// The first lint loads stylelint.config.mjs and its plugins: a one-time cold start that can pass the 5 s per-test
// default on a slow runner. Pay it once here (#455), so the cases below reuse the warm instance.
// Measured cold: about 50 ms locally.
beforeAll(async () => {
  await lintCss("margin: 0");
}, 60_000);

test.each([
  "color: #fff",
  "color: red",
  "background: red",
  "background: rgb(1 2 3)",
  "background-color: hsl(0 0% 0%)",
  "border: 1px solid red",
  "border-color: #000",
  "font: 600 17px/1.2 Arial",
  "font-size: 17px",
  "box-shadow: 0 0 0 2px red",
  "outline: 2px solid red",
  "padding: 10px",
  "margin: 0 0 0.5em",
  "gap: 3px",
  "border-radius: 7px",
  "border-top-left-radius: 7px",
  "inset: 3px",
  "height: 40px",
  "outline: none",
  "outline: 0",
  "margin: var(--nope)",
])("%s is rejected", async (decl) => {
  expect(await lintCss(decl)).not.toEqual([]);
});

test.each([
  "background: var(--panel)",
  "border: 1px solid var(--border)",
  "border: 1px solid transparent",
  "box-shadow: 0 0 0 2px var(--focus)",
  "font: 600 var(--text-s)/1.4 var(--font-ui)",
  "padding: 0 var(--space-4)",
  "margin: 0",
  "color: currentColor",
  "min-height: var(--control-h-m)",
  "font-size: var(--t3)",
  "outline: 3px solid var(--focus)",
])("%s passes", async (decl) => {
  expect(await lintCss(decl)).toEqual([]);
});

test("theme.css may hold literals", async () => {
  expect(await lintCss("--x: #fff; color: red", "src/theme.css")).toEqual([]);
});
