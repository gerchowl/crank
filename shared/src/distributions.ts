import type { Heatmap2D, Histogram, MetricId, MmpCurve } from "./contract";
import type { Athlete } from "./athlete";
import { normCdf } from "./util";

const MAX_BINS = 400;

/** Time-in-bin histogram from a 1 Hz series (NaN = no data). */
export function histogramFrom(metric: MetricId, series: readonly number[], binWidth: number, typicalOf?: (lo: number, hi: number) => number): Histogram {
  let bw = binWidth > 0 ? binWidth : 1;
  let lo = Infinity;
  let hi = -Infinity;
  let total = 0;
  for (const v of series) {
    if (!Number.isFinite(v)) continue;
    if (v < lo) lo = v;
    if (v > hi) hi = v;
    total++;
  }
  if (!total) return { metric, binWidth: bw, bins: [] };
  let lo0 = Math.floor(lo / bw) * bw;
  let nb = Math.floor((hi - lo0) / bw) + 1;
  if (nb > MAX_BINS) {
    bw = Math.ceil(((hi - lo0) / MAX_BINS) / bw) * bw;
    lo0 = Math.floor(lo / bw) * bw;
    nb = Math.floor((hi - lo0) / bw) + 1;
  }
  const secs = new Array<number>(nb).fill(0);
  for (const v of series) {
    if (!Number.isFinite(v)) continue;
    const i = Math.min(nb - 1, Math.floor((v - lo0) / bw));
    secs[i] = (secs[i] as number) + 1;
  }
  const h: Histogram = { metric, binWidth: bw, bins: secs.map((s, i) => ({ lo: round6(lo0 + i * bw), seconds: s })) };
  if (typicalOf) h.typical = secs.map((_, i) => typicalOf(lo0 + i * bw, lo0 + (i + 1) * bw) * total);
  return h;
}

const round6 = (x: number) => Math.round(x * 1e6) / 1e6;

/** Mixture-of-normals "typical ride" distribution for a metric (fraction of time in [lo, hi)). */
export function typicalDistribution(metric: MetricId, a: Athlete): ((lo: number, hi: number) => number) | undefined {
  const f = a.ftp;
  let comps: [number, number, number][]; // weight, mean, sd
  switch (metric) {
    case "power":
    case "power_3s":
    case "power_10s":
    case "power_30s":
      comps = [[0.12, 0.25 * f, 0.12 * f], [0.45, 0.62 * f, 0.07 * f], [0.28, 0.82 * f, 0.08 * f], [0.15, 1.05 * f, 0.1 * f]];
      break;
    case "pct_ftp":
      comps = [[0.12, 25, 12], [0.45, 62, 7], [0.28, 82, 8], [0.15, 105, 10]];
      break;
    case "hr":
      comps = [[0.2, 115, 10], [0.5, 138, 9], [0.3, 160, 9]];
      break;
    case "cadence":
      comps = [[0.1, 75, 8], [0.9, 89, 5]];
      break;
    default:
      return undefined;
  }
  return (lo, hi) => comps.reduce((s, [w, m, sd]) => s + w * (normCdf((hi - m) / sd) - normCdf((lo - m) / sd)), 0);
}

export function heatmapFrom(x: MetricId, y: MetricId, xs: readonly number[], ys: readonly number[], xBin: number, yBin: number): Heatmap2D {
  const xb = xBin > 0 ? xBin : 1;
  const yb = yBin > 0 ? yBin : 1;
  const n = Math.min(xs.length, ys.length);
  let xl = Infinity, xh = -Infinity, yl = Infinity, yh = -Infinity;
  for (let i = 0; i < n; i++) {
    const a = xs[i] as number;
    const b = ys[i] as number;
    if (!Number.isFinite(a) || !Number.isFinite(b)) continue;
    if (a < xl) xl = a;
    if (a > xh) xh = a;
    if (b < yl) yl = b;
    if (b > yh) yh = b;
  }
  if (!Number.isFinite(xl)) return { x, y, xBin: xb, yBin: yb, xLo: 0, yLo: 0, cols: 0, rows: 0, seconds: [] };
  const xLo = Math.floor(xl / xb) * xb;
  const yLo = Math.floor(yl / yb) * yb;
  const cols = Math.min(MAX_BINS, Math.floor((xh - xLo) / xb) + 1);
  const rows = Math.min(MAX_BINS, Math.floor((yh - yLo) / yb) + 1);
  const seconds = new Array<number>(rows * cols).fill(0);
  for (let i = 0; i < n; i++) {
    const a = xs[i] as number;
    const b = ys[i] as number;
    if (!Number.isFinite(a) || !Number.isFinite(b)) continue;
    const c = Math.min(cols - 1, Math.floor((a - xLo) / xb));
    const r = Math.min(rows - 1, Math.floor((b - yLo) / yb));
    seconds[r * cols + c] = (seconds[r * cols + c] as number) + 1;
  }
  return { x, y, xBin: xb, yBin: yb, xLo: round6(xLo), yLo: round6(yLo), cols, rows, seconds };
}

/** Log-spaced MMP durations 1 s … 3600 s. */
export const MMP_DURATIONS_S: readonly number[] = [1, 2, 3, 5, 7, 10, 15, 20, 30, 45, 60, 90, 120, 180, 240, 300, 420, 600, 900, 1200, 1800, 2400, 3600];

/** Best mean power for each duration from a 1 Hz series (missing = 0 W). 0 where the ride is shorter than d. */
export function mmpFrom(power: readonly number[], durations: readonly number[] = MMP_DURATIONS_S): number[] {
  const n = power.length;
  const pre = new Float64Array(n + 1);
  for (let i = 0; i < n; i++) {
    const v = power[i] as number;
    pre[i + 1] = (pre[i] as number) + (Number.isFinite(v) ? v : 0);
  }
  return durations.map((d) => {
    if (d > n) return 0;
    let best = 0;
    for (let i = d; i <= n; i++) {
      const m = ((pre[i] as number) - (pre[i - d] as number)) / d;
      if (m > best) best = m;
    }
    return Math.round(best * 10) / 10;
  });
}

/**
 * Plausible synthetic power-duration curve: 3-parameter CP model (Pmax, CP, W') with a gentle
 * decline beyond 20 min so that 60-min power ≈ FTP.
 */
export function syntheticMmp(a: Athlete, scale: number, durations: readonly number[] = MMP_DURATIONS_S): number[] {
  const cp = a.ftp * 1.04;
  const w = 20000;
  const pmax = a.ftp * 3.9;
  let prev = Infinity;
  return durations.map((d) => {
    let p = w / (d + w / (pmax - cp)) + cp;
    if (d > 1200) p *= 1 - 0.07 * Math.log10(d / 1200) / Math.log10(3);
    p *= scale;
    p = Math.min(prev, p);
    prev = p;
    return Math.round(p);
  });
}

export function emptyMmp(): MmpCurve {
  return { durationsS: [...MMP_DURATIONS_S], today: MMP_DURATIONS_S.map(() => 0), best90d: [], allTime: [] };
}
