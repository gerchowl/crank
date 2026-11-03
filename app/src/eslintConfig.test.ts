import { ESLint } from "eslint";
import { expect, test } from "vitest";

// Each rule of DESIGN §18.7 rules 1, 2, 4 and 7 that eslint.config.js enforces must fail on a violation (#455).
const eslint = new ESLint({ cwd: new URL("..", import.meta.url).pathname });
const lint = async (code: string, file = "src/x.tsx") => {
  const [r] = await eslint.lintText(code, { filePath: file });
  return r!.messages.map((m) => m.ruleId);
};
const jsx = (style: string) =>
  `export const A = () => <div style={${style}} />;\n`;

test.each([
  `{ color: "#fff" }`,
  `{ background: "rgb(1, 2, 3)" }`,
  `{ borderColor: "hsl(0 0% 0%)" }`,
  `{ color: "red" }`,
  `{ boxShadow: "0 0 4px var(--bg)" }`,
  `{ fill: "teal" }`,
  "{ color: `#f00` }",
  "{ color: `rgb(${n}, 0, 0)` }",
  "{ background: `linear-gradient(${n}deg, red, blue)` }",
  `{ borderTop: "solid red" }`,
  `{ outline: "1px solid blue" }`,
  `{ "color": "red" }`,
  `{ ["color"]: "#fff" }`,
  `{ stroke: "currentColor", fill: "#000" }`,
])("a colour literal in an inline style fails: %s", async (style) => {
  expect(await lint(jsx(style))).toContain("no-restricted-syntax");
});

test.each([
  `{ width: 12 }`,
  `{ width: "12px" }`,
  `{ margin: "0 8px" }`,
  `{ fontSize: 13.5 }`,
  "{ width: `${n}px` }",
  `{ margin: -8 }`,
  `{ margin: +8 }`,
  `{ outlineWidth: 3 }`,
  `{ strokeWidth: 2 }`,
  `{ gridTemplateColumns: "12px 1fr" }`,
  `{ width: n ? 8 : 4 }`,
  `{ "width": 8 }`,
])("a literal px in an inline style fails: %s", async (style) => {
  const code = `export const n = 3;\n${jsx(style)}`;
  expect(await lint(code)).toContain("no-restricted-syntax");
});

test.each([
  `{ color: "var(--ink)" }`,
  `{ background: "transparent" }`,
  `{ width: "7ch" }`,
  `{ height: "100%", flex: "1 1 0", minHeight: 0, flexGrow: 1 }`,
  "{ width: `${n}ch`, fontSize: 'var(--t3p)' }",
  `{ ...tokens, margin: 0 }`,
  `{ opacity: 0.5, zIndex: 2, flex: 1, fontWeight: 600, lineHeight: 1.5 }`,
  `{ "--ch": 7, stroke: "var(--red)", fill: "currentColor" }`,
  `{ border: "none", outline: "inherit", margin: -n }`,
])("a token, keyword or computed inline style passes: %s", async (style) => {
  const code = `export const n = 3;\nexport const tokens = {};\n${jsx(style)}`;
  expect(await lint(code)).toEqual([]);
});

test("a raw <svg> fails outside src/icons and passes inside", async () => {
  const code = `export const I = () => <svg viewBox="0 0 1 1" role="img" aria-label="x" />;\n`;
  expect(await lint(code)).toContain("no-restricted-syntax");
  expect(await lint(code, "src/icons/I.tsx")).toEqual([]);
});

test.each([
  `export const I = () => createElement("svg");`,
  `export const I = () => React.createElement("svg", null);`,
  `export const I = () => <i dangerouslySetInnerHTML={{ __html: "<svg/>" }} />;`,
  "export const I = () => <i dangerouslySetInnerHTML={{ __html: `<SVG>${1}` }} />;",
])("another way to make an <svg> fails: %s", async (code) => {
  const src = `import { createElement } from "react";\nimport React from "react";\n${code}\n`;
  expect(await lint(src)).toContain("no-restricted-syntax");
  expect(await lint(src, "src/icons/I.tsx")).not.toContain(
    "no-restricted-syntax",
  );
});

test.each(["react-icons/fa", "@heroicons/react/24/solid", "feather-icons"])(
  "an icon import from %s fails",
  async (pkg) => {
    const code = `import X from "${pkg}";\nexport const I = () => X;\n`;
    expect(await lint(code, "src/x.ts")).toContain("no-restricted-imports");
  },
);

test("lucide-react imports pass", async () => {
  const code = `import { Play } from "lucide-react";\nexport const I = () => <Play aria-hidden />;\n`;
  expect(await lint(code)).toEqual([]);
});

test("jsx-a11y is on: an img without alt fails", async () => {
  const code = `export const I = () => <img src="a.png" />;\n`;
  expect(await lint(code)).toContain("jsx-a11y/alt-text");
});

test("src/stream keeps both the structuredClone ban and the design rules", async () => {
  const code = `export const f = (x: unknown) => structuredClone(x);\nexport const A = () => <div style={{ color: "red" }} />;\n`;
  const rules = await lint(code, "src/stream/x.tsx");
  expect(rules.filter((r) => r === "no-restricted-syntax")).toHaveLength(2);
});
