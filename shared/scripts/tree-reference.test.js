import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { treeReference } from "./tree-reference";

const fixture = fileURLToPath(new URL("../../crates/crank-domain/tests/fixtures/tree_helpers.json", import.meta.url));

describe("tree helper reference fixture (#152, #159, #174)", () => {
  it("re-exports the committed reId/findNode fixture byte for byte", () => {
    const generated = treeReference();
    if (process.env.CRANK_EXPORT_VECTORS === "1") writeFileSync(fixture, generated);
    expect(readFileSync(fixture, "utf8"), "run just vectors to regenerate").toBe(generated);
  });
});
