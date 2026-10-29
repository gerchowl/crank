import { ESLint } from "eslint";
import { expect, test } from "vitest";

// The repo's own ESLint config, run on source text as if it lived at `filePath`.
const lint = async (code: string, filePath: string) => {
  const eslint = new ESLint({
    cwd: new URL("../..", import.meta.url).pathname,
  });
  const [result] = await eslint.lintText(code, { filePath });
  return result?.messages ?? [];
};

const FIXTURE = [
  "export const a = (x: unknown) => structuredClone(x);",
  "export const b = (x: unknown) => globalThis.structuredClone(x);",
  "export const c = (x: unknown) => window.structuredClone(x);",
  'export const d = (x: unknown) => globalThis["structuredClone"](x);',
  "export const e = (x: unknown) => globalThis[`structuredClone`](x);",
  "export const f = structuredClone;",
].join("\n");

test("structuredClone is an error in src/stream, however it is reached", async () => {
  const messages = await lint(FIXTURE, "src/stream/fixture.ts");
  const hits = messages.filter((m) => m.ruleId === "no-restricted-syntax");
  expect(hits.map((m) => m.line)).toEqual([1, 2, 3, 4, 5, 6]);
  expect(hits[0]?.message).toMatch(
    /structuredClone is forbidden in src\/stream/,
  );
});

test("the rule is scoped to src/stream", async () => {
  const messages = await lint(FIXTURE, "src/elsewhere.ts");
  expect(messages.filter((m) => m.ruleId === "no-restricted-syntax")).toEqual(
    [],
  );
});
