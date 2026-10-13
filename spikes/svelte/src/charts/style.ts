// Shared chart chrome: recessive grid/axes, ink for text, 2 px series lines.
import { timeAgoTicks, timeAgoToU, type Bucket, type TimeScale } from "@neo/shared";
import type uPlot from "uplot";
import type { ChartColors } from "../lib/theme.svelte";
import { dur } from "../lib/format";

export function axisStyle(c: ChartColors, extra: Partial<uPlot.Axis> = {}): uPlot.Axis {
  return {
    stroke: c.muted,
    font: `11px ${c.font}`,
    grid: { stroke: c.grid, width: 1 },
    ticks: { stroke: c.grid, width: 1, size: 3 },
    gap: 3,
    ...extra,
  };
}

/** x axis for log/fisheye time-ago charts: x = 1 − u (now at the right). */
export function timeAgoAxis(c: ChartColors, spanS: number, scale: TimeScale, show: boolean): uPlot.Axis {
  const ticks = timeAgoTicks(spanS);
  return axisStyle(c, {
    show,
    size: show ? 22 : 0,
    splits: () => ticks.map((t) => 1 - timeAgoToU(t, spanS, scale)),
    values: (_u, splits) => splits.map((x) => {
      const i = ticks.findIndex((t) => Math.abs(1 - timeAgoToU(t, spanS, scale) - x) < 1e-9);
      const t = ticks[i];
      return t === undefined ? "" : t === 0 ? "now" : dur(t);
    }),
  });
}

/**
 * Buckets → aligned [x, mean, min, max] with a null point at every gap so lines break where
 * data is missing (README: gap when prev.tAgo !== cur.tAgo + cur.dur).
 */
export function bucketsToSeries(buckets: readonly Bucket[], spanS: number, scale: TimeScale): [number[], (number | null)[], (number | null)[], (number | null)[]] {
  const xs: number[] = [];
  const mean: (number | null)[] = [];
  const mn: (number | null)[] = [];
  const mx: (number | null)[] = [];
  let prev: Bucket | undefined;
  for (const b of buckets) {
    if (prev && Math.abs(prev.tAgo - (b.tAgo + b.dur)) > 1e-6) {
      xs.push(1 - timeAgoToU(b.tAgo + b.dur, spanS, scale));
      mean.push(null);
      mn.push(null);
      mx.push(null);
    }
    xs.push(1 - timeAgoToU(b.tAgo + b.dur / 2, spanS, scale));
    mean.push(b.mean);
    mn.push(b.min);
    mx.push(b.max);
    prev = b;
  }
  return [xs, mean, mn, mx];
}

/** Map aligned series of another metric onto a reference x grid (nearest earlier x). */
export function alignTo(xs: readonly number[], other: readonly number[], vals: readonly (number | null)[]): (number | null)[] {
  const out: (number | null)[] = [];
  let j = 0;
  for (const x of xs) {
    while (j + 1 < other.length && (other[j + 1] ?? Infinity) <= x + 1e-9) j++;
    const ox = other[j];
    out.push(ox !== undefined && Math.abs(ox - x) < 0.02 ? (vals[j] ?? null) : null);
  }
  return out;
}

export function niceMax(v: number, step: number): number {
  return Math.ceil(v / step) * step;
}
