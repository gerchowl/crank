export const clamp = (x: number, lo: number, hi: number): number => (x < lo ? lo : x > hi ? hi : x);

export const lerp = (a: number, b: number, f: number): number => a + (b - a) * f;

export function deepClone<T>(x: T): T {
  return structuredClone(x);
}

let idCounter = 0;
const idPrefix = Math.floor(Math.random() * 36 ** 3).toString(36);

/** Fresh unique id (unique within this JS realm). Not deterministic across reloads. */
export function uid(prefix = "n"): string {
  idCounter += 1;
  return `${prefix}_${idPrefix}${idCounter.toString(36)}`;
}

/** Round to a sensible number of decimals for display/serialisation. */
export function round(x: number, decimals = 0): number {
  const f = 10 ** decimals;
  return Math.round(x * f) / f;
}

export function mean(xs: ArrayLike<number>): number {
  let s = 0;
  let n = 0;
  for (let i = 0; i < xs.length; i++) {
    const v = xs[i] as number;
    if (Number.isFinite(v)) {
      s += v;
      n++;
    }
  }
  return n ? s / n : NaN;
}

/** Standard normal CDF (Abramowitz-Stegun 7.1.26 based erf). */
export function normCdf(z: number): number {
  const x = Math.abs(z) / Math.SQRT2;
  const t = 1 / (1 + 0.3275911 * x);
  const y =
    1 - ((((1.061405429 * t - 1.453152027) * t + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t * Math.exp(-x * x);
  return z >= 0 ? 0.5 * (1 + y) : 0.5 * (1 - y);
}
