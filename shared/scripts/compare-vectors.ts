import type { VectorSchema } from "./vector-schema";

type Kind = "float" | "integer" | "exact";
/** A schema node with references resolved and the shorthand string kinds expanded. */
interface Node {
  kind: Kind | undefined;
  /** Float scalar allowed at a node that also has object/array variants. */
  floatScalar: boolean;
  nullable: boolean;
  fields: Record<string, VectorSchema> | undefined;
  items: VectorSchema | undefined;
  indexed: VectorSchema | undefined;
}

const NON_FINITE = new Set(["NaN", "Infinity", "-Infinity"]);
const isRecord = (value: unknown): value is Record<string, unknown> =>
  value !== null && typeof value === "object" && !Array.isArray(value);

const expand = (schema: VectorSchema): Node =>
  typeof schema === "string"
    ? { kind: schema, floatScalar: false, nullable: false, fields: undefined, items: undefined, indexed: undefined }
    : { kind: schema.kind, floatScalar: schema.number === "float", nullable: schema.nullable === true,
        fields: schema.fields, items: schema.items, indexed: schema.indexed };

const child = (node: Node, key: string): VectorSchema | undefined => {
  if (node.fields && Object.hasOwn(node.fields, key)) return node.fields[key];
  if (node.indexed && /^(0|[1-9][0-9]*)$/.test(key)) return node.indexed;
  return undefined;
};

/** First semantic JSON mismatch; unknown schema fields throw, even for equal inputs. */
export function vectorDifference(
  expected: unknown, actual: unknown, schema: VectorSchema,
  definitions: Record<string, VectorSchema> = {},
): string | null {
  const resolve = (schema: VectorSchema | undefined, path: string): Node => {
    if (schema === undefined) throw new Error(`${path}: field missing from schema`);
    const node = expand(schema);
    if (typeof schema === "string" || schema.ref === undefined) return node;
    const target = definitions[schema.ref];
    if (target === undefined) throw new Error(`${path}: unknown schema reference ${schema.ref}`);
    const resolved = expand(target);
    return { ...resolved, nullable: resolved.nullable || node.nullable };
  };
  // Check both complete documents before returning a difference: an earlier
  // mismatch must not conceal an unclassified field later in either document.
  const validate = (value: unknown, schema: VectorSchema | undefined, path: string): void => {
    const node = resolve(schema, path);
    if (value === null) {
      if (!node.nullable) throw new Error(`${path}: null at a node not declared nullable`);
    } else if (typeof value === "number") {
      if (node.kind !== "float" && node.kind !== "integer" && !node.floatScalar) {
        throw new Error(`${path}: numeric kind missing from schema`);
      }
    } else if (typeof value === "string") {
      const floating = node.kind === "float" || node.floatScalar;
      if (node.kind !== "exact" && !(floating && NON_FINITE.has(value))) {
        throw new Error(`${path}: string at a node declared ${node.kind ?? "structured"}, only non-finite sentinels are allowed there`);
      }
    } else if (typeof value === "boolean") {
      if (node.kind !== "exact") throw new Error(`${path}: boolean at a node not declared exact`);
    } else if (Array.isArray(value)) {
      const { items } = node;
      if (items === undefined) throw new Error(`${path}: array missing from schema`);
      value.forEach((item, index) => validate(item, items, `${path}[${index}]`));
    } else if (isRecord(value)) {
      if (!node.fields && !node.indexed) throw new Error(`${path}: object missing from schema`);
      for (const [key, item] of Object.entries(value)) validate(item, child(node, key), `${path}[${JSON.stringify(key)}]`);
    }
  };
  validate(expected, schema, "$");
  validate(actual, schema, "$");
  const compare = (expected: unknown, actual: unknown, schema: VectorSchema | undefined, path: string): string | null => {
    const node = resolve(schema, path);
    if (typeof expected === "number" && typeof actual === "number") {
      if (Number.isFinite(expected) && Number.isFinite(actual)) {
        if (node.kind === "integer") {
          if (expected === actual && Number.isInteger(expected)) return null;
        } else {
          const tolerance = Math.max(1e-12 * Math.max(Math.abs(expected), Math.abs(actual)), 1e-15);
          if (Math.abs(expected - actual) <= tolerance) return null;
        }
      }
      return `${path}: expected ${expected}, got ${actual}`;
    }
    if (expected === actual && (expected === null || typeof expected !== "object")) return null;
    if (Array.isArray(expected) && Array.isArray(actual)) {
      if (expected.length !== actual.length) return `${path}: array lengths ${expected.length} and ${actual.length} differ`;
      for (let i = 0; i < expected.length; i++) {
        const difference = compare(expected[i], actual[i], node.items, `${path}[${i}]`);
        if (difference !== null) return difference;
      }
      return null;
    }
    if (isRecord(expected) && isRecord(actual)) {
      const expectedKeys = Object.keys(expected).sort();
      const actualKeys = Object.keys(actual).sort();
      if (expectedKeys.length !== actualKeys.length || expectedKeys.some((key, i) => key !== actualKeys[i])) {
        return `${path}: object keys differ`;
      }
      for (const key of expectedKeys) {
        const difference = compare(expected[key], actual[key], child(node, key), `${path}[${JSON.stringify(key)}]`);
        if (difference !== null) return difference;
      }
      return null;
    }
    return `${path}: values or types differ`;
  };
  return compare(expected, actual, schema, "$");
}
