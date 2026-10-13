import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { timeAgoEdges, type TimeScale } from "../../shared/src/scales";

it("TS scales agree with Rust-exported history bucket edges", () => {
  const path = process.env.CRANK_SCALE_EDGES;
  const committed = JSON.parse(
    readFileSync(new URL("./rust-scale-edges.json", import.meta.url), "utf8"),
  ) as { spanS: number; buckets: number; scale: TimeScale; edges: number[] }[];
  const cases: typeof committed = path
    ? JSON.parse(readFileSync(path, "utf8"))
    : committed;
  // The flake supplies a fresh Rust export. Validate the committed export's
  // structure and numbers directly, using ADR-0001's portable float tolerance.
  expect(
    cases.map(({ edges, ...metadata }) => ({
      ...metadata,
      length: edges.length,
    })),
  ).toEqual(
    committed.map(({ edges, ...metadata }) => ({
      ...metadata,
      length: edges.length,
    })),
  );
  cases.forEach((c, k) =>
    c.edges.forEach((edge, i) => {
      const saved = committed[k]!.edges[i]!;
      expect(Math.abs(edge - saved)).toBeLessThanOrEqual(
        Math.max(1e-12 * Math.max(Math.abs(edge), Math.abs(saved)), 1e-15),
      );
    }),
  );
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
