import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

/**
 * #578, ADR-0005 §2.1 "One zero": a route the core sends survives the UI's JSON bit for bit.
 * The fixture is the core's JSON of `crates/crank-domain/tests/fixtures/gpx/negative-zero.gpx`
 * (pinned by `route_one_zero.rs`), a file written with `-0` elevations and `lon="-0.000000"`.
 */
const fixture = new URL(
  "../../crates/crank-domain/tests/fixtures/route-negative-zero.json",
  import.meta.url,
);

/** Every number of a JSON value, in document order. */
function numbers(v: unknown, out: number[] = []): number[] {
  if (typeof v === "number") out.push(v);
  else if (Array.isArray(v)) v.forEach((x) => numbers(x, out));
  else if (v !== null && typeof v === "object")
    Object.values(v).forEach((x) => numbers(x, out));
  return out;
}

it("a file route written with -0 round-trips through JavaScript's JSON", () => {
  const received: unknown = JSON.parse(readFileSync(fixture, "utf8"));
  // What the page would send back to the core, read as the core reads it.
  const sentBack: unknown = JSON.parse(JSON.stringify(received));
  const before = numbers(received);
  const after = numbers(sentBack);
  expect(after).toHaveLength(before.length);
  // Object.is tells -0 from 0, as the core's bit-for-bit comparison does.
  const show = (x: number | undefined): string =>
    Object.is(x, -0) ? "-0" : String(x);
  const changed = before.flatMap((x, i) =>
    Object.is(x, after[i])
      ? []
      : [`number ${i}: ${show(x)} became ${show(after[i])}`],
  );
  expect(changed).toEqual([]);
  expect(before.some((x) => Object.is(x, -0))).toBe(false);
});
