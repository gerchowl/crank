/** First semantic JSON mismatch, or null; float parity follows ADR-0001 §3. */
export function vectorDifference(expected: unknown, actual: unknown, path = "$"): string | null {
  if (typeof expected === "number" && typeof actual === "number") {
    // Integers are exact, including when only one operand is an integer.
    if (Number.isFinite(expected) && Number.isFinite(actual)) {
      if (Number.isInteger(expected) || Number.isInteger(actual)) {
        if (expected === actual) return null;
      } else {
        // JS transcendental results vary by platform/libm. Keep the exported
        // precision and apply the architect's tolerance only at comparison.
        const tolerance = Math.max(1e-12 * Math.max(Math.abs(expected), Math.abs(actual)), 1e-15);
        if (Math.abs(expected - actual) <= tolerance) return null;
      }
    }
    return `${path}: expected ${expected}, got ${actual}`;
  }
  if (expected === actual) return null;
  if (Array.isArray(expected) && Array.isArray(actual)) {
    if (expected.length !== actual.length) return `${path}: array lengths ${expected.length} and ${actual.length} differ`;
    for (let i = 0; i < expected.length; i++) {
      const difference = vectorDifference(expected[i], actual[i], `${path}[${i}]`);
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
      const difference = vectorDifference(
        (expected as Record<string, unknown>)[key], (actual as Record<string, unknown>)[key], `${path}[${JSON.stringify(key)}]`,
      );
      if (difference !== null) return difference;
    }
    return null;
  }
  return `${path}: values or types differ`;
}
