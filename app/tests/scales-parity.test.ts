import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { timeAgoEdges, type TimeScale } from "../../shared/src/scales";

it("TS scales agree with Rust-exported history bucket edges", () => {
  const path = process.env.CRANK_SCALE_EDGES;
  // This cross-language gate runs with freshly exported Rust edges in the flake.
  // Ordinary app-test also checks the committed Rust export for drift in TS.
  const cases = JSON.parse(
    readFileSync(
      path ?? new URL("./rust-scale-edges.json", import.meta.url),
      "utf8",
    ),
  ) as { spanS: number; buckets: number; scale: TimeScale; edges: number[] }[];
  expect(cases).toHaveLength(60);
  for (const c of cases) {
    const edges = timeAgoEdges(c.spanS, c.buckets, c.scale);
    expect(edges).toHaveLength(c.edges.length);
    edges.forEach((edge, i) => {
      const rust = c.edges[i]!;
      expect(Math.abs(edge - rust)).toBeLessThanOrEqual(
        Math.max(1e-12 * Math.max(Math.abs(edge), Math.abs(rust)), 1e-15),
      );
    });
  }
});
