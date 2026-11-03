// Test helper: tests that read local time pin the zone and put it back, so it cannot leak into another test file
// when the worker is reused.

/** Sets `TZ` and returns what puts the previous value (or its absence) back. */
export function setTimeZone(tz: string): () => void {
  const had = Object.prototype.hasOwnProperty.call(process.env, "TZ");
  const previous = process.env.TZ;
  process.env.TZ = tz;
  return () => {
    if (had) process.env.TZ = previous;
    else delete process.env.TZ;
  };
}

/** Runs `fn` under `tz`; the zone is restored when it returns or throws. */
export async function withTimeZone<T>(
  tz: string,
  fn: () => T | Promise<T>,
): Promise<T> {
  const restore = setTimeZone(tz);
  try {
    return await fn();
  } finally {
    restore();
  }
}
