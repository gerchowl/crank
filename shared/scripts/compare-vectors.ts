import type { VectorSchema } from "./vector-schema";

const field = (schema: VectorSchema, key: string): VectorSchema | undefined => {
  if (typeof schema === "string") return undefined;
  if (schema.fields && Object.hasOwn(schema.fields, key)) return schema.fields[key];
  if (schema.indexed && /^(0|[1-9][0-9]*)$/.test(key)) return schema.indexed;
  return undefined;
};

/** First semantic JSON mismatch; unknown schema fields throw, even for equal inputs. */
export function vectorDifference(
  expected: unknown, actual: unknown, schema: VectorSchema,
  definitions: Record<string, VectorSchema> = {},
): string | null {
  const resolve = (node: VectorSchema | undefined, path: string): VectorSchema => {
    if (node === undefined) throw new Error(`${path}: field missing from schema`);
    if (typeof node !== "string" && node.ref !== undefined) {
      const target = definitions[node.ref];
      if (target === undefined) throw new Error(`${path}: unknown schema reference ${node.ref}`);
      return target;
    }
    return node;
  };
  // Check both complete documents before returning a difference: an earlier
  // mismatch must not conceal an unclassified field later in either document.
  const validate = (value: unknown, node: VectorSchema | undefined, path: string): void => {
    const shape = resolve(node, path);
    if (typeof value === "number") {
      if (shape !== "float" && shape !== "integer" && !(typeof shape !== "string" && shape.number)) {
        throw new Error(`${path}: numeric kind missing from schema`);
      }
    } else if (Array.isArray(value)) {
      if (typeof shape === "string" || shape.items === undefined) throw new Error(`${path}: array missing from schema`);
      value.forEach((item, index) => validate(item, shape.items, `${path}[${index}]`));
    } else if (value !== null && typeof value === "object") {
      if (typeof shape === "string" || (!shape.fields && !shape.indexed)) throw new Error(`${path}: object missing from schema`);
      for (const [key, item] of Object.entries(value)) validate(item, field(shape, key), `${path}[${JSON.stringify(key)}]`);
    }
  };
  validate(expected, schema, "$");
  validate(actual, schema, "$");
  const compare = (expected: unknown, actual: unknown, node: VectorSchema, path: string): string | null => {
    const shape = resolve(node, path);
    if (typeof expected === "number" && typeof actual === "number") {
      if (Number.isFinite(expected) && Number.isFinite(actual)) {
        if (shape === "integer") {
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
        const difference = compare(expected[i], actual[i], (shape as { items: VectorSchema }).items, `${path}[${i}]`);
        if (difference !== null) return difference;
      }
      return null;
    }
    if (expected !== null && actual !== null && typeof expected === "object" && typeof actual === "object"
        && !Array.isArray(expected) && !Array.isArray(actual)) {
      const expectedKeys = Object.keys(expected).sort();
      const actualKeys = Object.keys(actual).sort();
      if (expectedKeys.length !== actualKeys.length || expectedKeys.some((key, i) => key !== actualKeys[i])) {
        return `${path}: object keys differ`;
      }
      for (const key of expectedKeys) {
        const difference = compare(
          (expected as Record<string, unknown>)[key], (actual as Record<string, unknown>)[key], field(shape, key)!, `${path}[${JSON.stringify(key)}]`,
        );
        if (difference !== null) return difference;
      }
      return null;
    }
    return `${path}: values or types differ`;
  };
  return compare(expected, actual, schema, "$");
}
