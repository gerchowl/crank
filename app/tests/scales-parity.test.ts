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
    expectEdges(c.edges, committed[k]!.edges, `saved case ${k}`),
  );
  expect(cases).toHaveLength(60);
  for (const c of cases) {
    const edges = timeAgoEdges(c.spanS, c.buckets, c.scale);
    expect(edges).toHaveLength(c.edges.length);
    expectEdges(
      edges,
      c.edges,
      `${c.scale}, span=${c.spanS}, buckets=${c.buckets}`,
    );
  }
});

function expectEdges(actual: number[], expected: number[], context: string) {
  // Keep every numeric comparison and its tolerance, but invoke the matcher
  // once per case. Per-edge matcher construction dominated this 50k-comparison test.
  const mismatches: {
    index: number;
    actual: number;
    expected: number;
    tolerance: number;
  }[] = [];
  actual.forEach((edge, index) => {
    const saved = expected[index]!;
    const tolerance = Math.max(
      1e-12 * Math.max(Math.abs(edge), Math.abs(saved)),
      1e-15,
    );
    if (!(Math.abs(edge - saved) <= tolerance)) {
      mismatches.push({ index, actual: edge, expected: saved, tolerance });
    }
  });
  expect(mismatches, context).toEqual([]);
}
