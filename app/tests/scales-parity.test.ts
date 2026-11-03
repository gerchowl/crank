import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { timeAgoEdges, type TimeScale } from "../../shared/src/scales";

/** ADR-0001 portable float tolerance (relative). */
const TOLERANCE = 1e-12;

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
  expect(cases).toHaveLength(60);
  // One assertion per comparison costs ~5 us each on a warm machine, ~50k of
  // them (two per edge) made the test the slowest in the suite and flaky
  // against Vitest's 5 s default on macOS CI (#353). Compare in plain loops
  // and assert once on the list of disagreements, which also names them.
  const disagreements: string[] = [];
  const compare = (what: string, got: number, want: number): void => {
    if (
      !(
        Math.abs(got - want) <=
        Math.max(TOLERANCE * Math.max(Math.abs(got), Math.abs(want)), 1e-15)
      )
    ) {
      disagreements.push(`${what}: ${got} vs ${want}`);
    }
  };
  cases.forEach((c, k) =>
    c.edges.forEach((edge, i) =>
      compare(`committed ${k}[${i}]`, edge, committed[k]!.edges[i]!),
    ),
  );
  for (const [k, c] of cases.entries()) {
    const edges = timeAgoEdges(c.spanS, c.buckets, c.scale);
    expect(edges).toHaveLength(c.edges.length);
    edges.forEach((edge, i) =>
      compare(
        `${c.scale} span ${c.spanS} buckets ${c.buckets} case ${k}[${i}]`,
        edge,
        c.edges[i]!,
      ),
    );
  }
  expect(disagreements).toEqual([]);
});
