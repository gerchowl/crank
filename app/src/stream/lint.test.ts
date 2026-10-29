import { readFileSync } from "node:fs";
import { ESLint } from "eslint";
import { beforeAll, expect, test } from "vitest";

// One ESLint instance for the file with a minimal inline config: only the structuredClone restriction, the
// default parser (so the fixtures are plain JavaScript: no TypeScript parser, no type information, no plugins).
// Loading the repo's full config and plugin set per case took 7 s on the macOS runner against Vitest's 5 s default
// (#319). Because the rule is copied here, the last test pins the copy to `eslint.config.js`.
const SELECTORS = [
  "Identifier[name='structuredClone']",
  "Literal[value='structuredClone']",
  "TemplateLiteral[quasis.length=1][quasis.0.value.cooked='structuredClone']",
];
const MESSAGE =
  "structuredClone is forbidden in src/stream: stream data is plain JSON, replace it, do not clone it.";
const STREAM_FILES = "src/stream/**/*.{ts,tsx}";

let eslint: ESLint;
beforeAll(() => {
  eslint = new ESLint({
    cwd: new URL("../..", import.meta.url).pathname,
    overrideConfigFile: true,
    overrideConfig: [
      {
        files: [STREAM_FILES],
        rules: {
          "no-restricted-syntax": [
            "error",
            ...SELECTORS.map((selector) => ({ selector, message: MESSAGE })),
          ],
        },
      },
    ],
  });
});

// The minimal config, run on source text as if it lived at `filePath`.
const lint = async (code: string, filePath: string) => {
  const [result] = await eslint.lintText(code, { filePath });
  return result?.messages ?? [];
};

const FIXTURE = [
  "export const a = (x) => structuredClone(x);",
  "export const b = (x) => globalThis.structuredClone(x);",
  "export const c = (x) => window.structuredClone(x);",
  'export const d = (x) => globalThis["structuredClone"](x);',
  "export const e = (x) => globalThis[`structuredClone`](x);",
  "export const f = structuredClone;",
].join("\n");

test("structuredClone is an error in src/stream, however it is reached", async () => {
  const messages = await lint(FIXTURE, "src/stream/fixture.ts");
  expect(messages.filter((m) => m.fatal)).toEqual([]); // the fixtures parse
  const hits = messages.filter((m) => m.ruleId === "no-restricted-syntax");
  expect(hits.map((m) => m.line)).toEqual([1, 2, 3, 4, 5, 6]);
  expect(hits[0]?.message).toMatch(
    /structuredClone is forbidden in src\/stream/,
  );
});

test("the rule is scoped to src/stream", async () => {
  const messages = await lint(FIXTURE, "src/elsewhere.ts");
  expect(messages.filter((m) => m.fatal)).toEqual([]);
  expect(messages.filter((m) => m.ruleId === "no-restricted-syntax")).toEqual(
    [],
  );
});

test("the inline rule is the one eslint.config.js enforces", () => {
  const config = readFileSync(
    new URL("../../eslint.config.js", import.meta.url),
    "utf8",
  );
  expect(config).toContain(STREAM_FILES);
  expect(config).toContain(MESSAGE);
  for (const selector of SELECTORS) expect(config).toContain(selector);
});
