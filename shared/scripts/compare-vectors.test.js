import { describe, expect, it } from "vitest";
import { vectorDifference } from "./compare-vectors";

describe("semantic vector comparison (ADR-0001 §3)", () => {
  it.each([
    [305.0239643501903, 305.0239643501902], // observed platform difference: one ulp
    [1.25, 1.25 + 1e-12], // relative term dominates
    [-1.25, -1.25 - 1e-12],
    [1e-16, 9e-16], // absolute floor dominates near zero
    [Number.MIN_VALUE, 2 * Number.MIN_VALUE],
    [42, 42], ["NaN", "NaN"], [true, true], [null, null],
    [{ a: [305.0239643501903], b: false }, { b: false, a: [305.0239643501902] }],
  ])("accepts permitted parity: %j / %j", (expected, actual) => {
    expect(vectorDifference(expected, actual)).toBeNull();
    expect(vectorDifference(actual, expected)).toBeNull();
  });

  it.each([
    [305.02, 305.03], // real change, not a platform rounding difference
    [1.25, 1.25 + 2e-12], // outside relative tolerance
    [1e-16, 2e-15], // outside absolute floor
    [1e12, 1e12 + 1], // integer difference within the float tolerance must fail
    [1, 1 + Number.EPSILON], // one integer operand still requires exactness
    ["1", 1], [true, 1], [false, true], [null, {}],
    [[1, 2], [2, 1]], [[1], [1, 2]], [[], {}],
    [{ a: null }, {}], [{ a: 1 }, { b: 1 }],
    ["NaN", "Infinity"], [NaN, NaN], [Infinity, Infinity],
  ])("rejects semantic drift: %j / %j", (expected, actual) => {
    expect(vectorDifference(expected, actual)).not.toBeNull();
    expect(vectorDifference(actual, expected)).not.toBeNull();
  });

  it("reports the path to a nested mismatch", () => {
    expect(vectorDifference({ cases: [{ output: 305.02 }] }, { cases: [{ output: 305.03 }] }))
      .toBe('$["cases"][0]["output"]: expected 305.02, got 305.03');
  });
});
