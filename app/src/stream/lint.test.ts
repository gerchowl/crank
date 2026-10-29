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

// The same six forms with TypeScript annotations, for the real config (its parser is typescript-eslint).
const TS_FIXTURE = FIXTURE.replaceAll("= (x) =>", "= (x: unknown) =>");

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

// The inline tests above prove the rule's selectors; this one proves the REAL `eslint.config.js` applies exactly that
// rule to src/stream: same rule and severity after the whole config is resolved (a later block, a changed severity
// or an `ignores` entry would all show here), not ignored, and parsed by the project's typescript-eslint parser.
// It loads the full config and plugin set, the expensive part the other tests avoid (#319): about 0.4 s locally, and
// the same load took 7.06 s on the macOS runner (#319), over Vitest's 5 s default. So this test, and only this one,
// gets 30 s.
test("the real eslint.config.js applies the same rule to src/stream", async () => {
  const cwd = new URL("../..", import.meta.url).pathname;
  const real = new ESLint({ cwd });
  const file = "src/stream/x.ts";

  expect(await real.isPathIgnored(file)).toBe(false);
  const config = await real.calculateConfigForFile(file);
  const expected = (await eslint.calculateConfigForFile(file)) as {
    rules: Record<string, unknown>;
  };
  expect(config.rules["no-restricted-syntax"]).toEqual(
    expected.rules["no-restricted-syntax"],
  );
  expect(config.languageOptions.parser.meta.name).toBe(
    "typescript-eslint/parser",
  );

  // and the six forms, as TypeScript, through the real config
  const [result] = await real.lintText(TS_FIXTURE, { filePath: file });
  const hits = result?.messages.filter(
    (m) => m.ruleId === "no-restricted-syntax",
  );
  expect(result?.messages.filter((m) => m.fatal)).toEqual([]);
  expect(hits?.map((m) => m.line)).toEqual([1, 2, 3, 4, 5, 6]);
}, 30_000);
