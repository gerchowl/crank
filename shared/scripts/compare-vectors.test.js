import { describe, expect, it } from "vitest";
import { vectorDifference } from "./compare-vectors";
import { vectorSchemas } from "./vector-schema";

const schema = { fields: { value: { kind: "float", nullable: true }, strict: "float", count: "integer", label: "exact",
  maybeLabel: { kind: "exact", nullable: true }, values: { items: "float" } } };
const difference = (a, b) => vectorDifference(a, b, schema);

describe("semantic vector comparison (ADR-0001 §3)", () => {
  it.each([
    [1, 1 + Number.EPSILON], [250, 250.00000000000003],
    [305.0239643501903, 305.0239643501902],
    [1.25, 1.25 + 1e-12], [-1.25, -1.25 - 1e-12],
    [1e-16, 9e-16], [Number.MIN_VALUE, 2 * Number.MIN_VALUE],
  ])("accepts float parity regardless of whole values: %j / %j", (a, b) => {
    expect(difference({ value: a }, { value: b })).toBeNull();
    expect(difference({ value: b }, { value: a })).toBeNull();
  });

  it.each([
    [{ count: 3 }, { count: 3.0000000000001 }],
    [{ count: 1e12 }, { count: 1e12 + 1 }],
    [{ count: 3.1 }, { count: 3.1 }],
    [{ value: 305.02 }, { value: 305.03 }],
    [{ value: 1.25 }, { value: 1.25 + 2e-12 }],
    [{ value: 1e-16 }, { value: 2e-15 }],
    [{ value: NaN }, { value: NaN }], [{ value: Infinity }, { value: Infinity }],
    [{ label: false }, { label: true }], [{ label: "NaN" }, { label: "Infinity" }],
    [{ value: null }, { value: 0 }], [{ values: [1, 2] }, { values: [2, 1] }],
    [{ values: [1] }, { values: [1, 2] }], [{ value: null }, {}],
  ])("rejects semantic drift: %j / %j", (a, b) => {
    expect(difference(a, b)).not.toBeNull();
    expect(difference(b, a)).not.toBeNull();
  });

  it("accepts exact discrete values, encoded non-finites, and reordered object keys", () => {
    expect(difference({ count: 42, label: true, value: null }, { value: null, label: true, count: 42 })).toBeNull();
    for (const value of ["NaN", "Infinity", "-Infinity"]) expect(difference({ value }, { value })).toBeNull();
  });

  it.each([
    [{ value: true }, "$[\"value\"]: boolean at a node not declared exact"],
    [{ count: false }, "$[\"count\"]: boolean at a node not declared exact"],
    [{ value: "1" }, "$[\"value\"]: string at a node declared float"],
    [{ count: "NaN" }, "$[\"count\"]: string at a node declared integer"],
    [{ strict: null }, "$[\"strict\"]: null at a node not declared nullable"],
    [{ values: [1, null] }, "$[\"values\"][1]: null at a node not declared nullable"],
    [{ label: null }, "$[\"label\"]: null at a node not declared nullable"],
  ])("rejects a value the declared kind cannot hold, even when both sides agree: %j", (value, message) => {
    expect(() => difference(value, value)).toThrow(message);
  });

  it("accepts null only where declared nullable", () => {
    expect(difference({ value: null, maybeLabel: null }, { value: null, maybeLabel: null })).toBeNull();
    expect(difference({ value: null }, { value: 1 })).not.toBeNull();
    expect(vectorDifference({ item: null }, { item: null }, { fields: { item: { ref: "node", nullable: true } } },
      { node: { fields: { x: "float" } } })).toBeNull();
  });

  it("reports the path to a nested mismatch", () => {
    expect(difference({ values: [305.02] }, { values: [305.03] }))
      .toBe('$["values"][0]: expected 305.02, got 305.03');
  });

  it.each([
    [{ unknown: 1 }, { unknown: 1 }],
    [{ value: 1 }, { value: 2, unknown: null }],
    [{ unknown: false }, {}],
  ])("errors for unknown fields on either side, including equal values", (a, b) => {
    expect(() => difference(a, b)).toThrow('field missing from schema');
    expect(() => difference(b, a)).toThrow('field missing from schema');
  });

  it("requires a numeric classification instead of defaulting exact fields", () => {
    expect(() => difference({ label: 1 }, { label: 1 })).toThrow('numeric kind missing from schema');
  });

  it("uses the exported schema for a compiler power float and tree index integer", () => {
    const document = (power, index) => ({ schemaVersion: 1, cases: [{ name: "compile", input: {},
      output: { blocks: [{ powerFrom: power, path: [index] }] } }] });
    const compare = (a, b) => vectorDifference(a, b, vectorSchemas.areas.compile, vectorSchemas.definitions);
    expect(compare(document(250, 3), document(250.00000000000003, 3))).toBeNull();
    expect(compare(document(250, 3), document(250, 3.0000000000001))).not.toBeNull();
  });

  it("checks integer validity inside an identical object", () => {
    const value = { count: 3.1 };
    expect(difference(value, value)).not.toBeNull();
  });

  it.each([
    ["compile", { timeInZoneS: [3] }, { timeInZoneS: [3.0000000000001] }],
    ["distributions", { bins: [{ lo: 250, seconds: 3 }] }, { bins: [{ lo: 250, seconds: 3.0000000000001 }] }],
    ["distributions", { seconds: [3] }, { seconds: [3.0000000000001] }],
  ])("keeps 1 Hz sample counts exact in %s", (area, expected, actual) => {
    const document = (output) => ({ schemaVersion: 1, cases: [{ name: "counts", input: {}, output }] });
    expect(vectorDifference(document(expected), document(actual), vectorSchemas.areas[area], vectorSchemas.definitions))
      .not.toBeNull();
  });

  it("resolves recursive declarations and rejects unknown nested fields", () => {
    const tree = [{ count: 3, children: [{ count: 2 }] }];
    const schema = { items: { ref: "node" } };
    expect(vectorDifference(tree, tree, schema, vectorSchemas.definitions)).toBeNull();
    tree[0].children[0].unknown = null;
    expect(() => vectorDifference(tree, tree, schema, vectorSchemas.definitions))
      .toThrow('$[0]["children"][0]["unknown"]: field missing from schema');
    expect(() => vectorDifference([], [], { ref: "absent" }))
      .toThrow('unknown schema reference absent');
  });

  it("classifies typed-array JSON entries and rejects non-index fields", () => {
    const schema = { number: "float", indexed: "float" };
    expect(vectorDifference({ 0: 250 }, { 0: 250.00000000000003 }, schema)).toBeNull();
    expect(() => vectorDifference({ extra: 250 }, { extra: 250 }, schema))
      .toThrow('field missing from schema');
  });
});
