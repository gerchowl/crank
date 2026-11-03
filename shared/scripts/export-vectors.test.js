import { mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { exportVectors, serializeVectors } from "./vectors";
import { vectorSchemas } from "./vector-schema";
import { vectorDifference } from "./compare-vectors";

const destination = fileURLToPath(new URL("../../crates/crank-domain/tests/vectors/", import.meta.url));

describe("golden vectors (ADR-0001 §3, #31)", () => {
  it("re-exports every committed JSON with semantic parity", () => {
    const vectors = exportVectors();
    const names = Object.keys(vectors).map((area) => `${area}.json`).sort();
    expect(names).toEqual([
      "athlete.json", "compile.json", "distributions.json", "dsl.json", "filters.json",
      "history.json", "hrv.json", "recovery.json", "rewrite.json", "simulator.json", "wbal.json",
    ]);
    if (process.env.CRANK_EXPORT_VECTORS === "1") mkdirSync(destination, { recursive: true });
    const schemaPath = `${destination}/schema.json`;
    const generatedSchema = JSON.stringify(vectorSchemas, null, 2) + "\n";
    if (process.env.CRANK_EXPORT_VECTORS === "1") writeFileSync(schemaPath, generatedSchema);
    expect(readFileSync(schemaPath, "utf8")).toBe(generatedSchema);
    const schema = JSON.parse(readFileSync(schemaPath, "utf8"));
    for (const [area, cases] of Object.entries(vectors)) {
      expect(cases.length, area).toBeGreaterThan(0);
      const generated = serializeVectors(cases);
      const path = `${destination}/${area}.json`;
      if (process.env.CRANK_EXPORT_VECTORS === "1") writeFileSync(path, generated);
      // ADR-0001 §3: exact structure/discrete values; full-precision floats
      // compare within max(1e-12 * max(|a|, |b|), 1e-15), without export rounding.
      expect(vectorDifference(JSON.parse(readFileSync(path, "utf8")), JSON.parse(generated), schema.areas[area], schema.definitions),
        `${area}: run just vectors to regenerate`).toBeNull();
    }
    expect(readdirSync(destination).filter((name) => name.endsWith(".json")).sort()).toEqual([...names, "schema.json"].sort());
  });

  it("preserves non-finite inputs distinctly from missing samples", () => {
    const serialized = serializeVectors([{ name: "encoding", input: [NaN, Infinity, -Infinity, null], output: 0 }]);
    expect(JSON.parse(serialized).cases[0].input).toEqual(["NaN", "Infinity", "-Infinity", null]);
  });
});
