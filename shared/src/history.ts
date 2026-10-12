import type { Bucket, MetricId } from "./contract";
import { timeAgoEdges, type TimeScale } from "./scales";

/** Resolution levels of the multi-resolution store: slot width (s) and ring capacity (slots). */
export const HISTORY_LEVELS: readonly { res: number; cap: number }[] = [
  { res: 1, cap: 1200 },   // 20 min at 1 s
  { res: 5, cap: 1440 },   // 2 h at 5 s
  { res: 30, cap: 480 },   // 4 h at 30 s
  { res: 300, cap: 288 },  // 24 h at 5 min
];

class Level {
  readonly slot: Float64Array;
  readonly sum: Float64Array;
  readonly cnt: Uint32Array;
  readonly min: Float64Array;
  readonly max: Float64Array;
  latest = -1;
  readonly res: number;
  readonly cap: number;

  constructor(res: number, cap: number) {
    this.res = res;
    this.cap = cap;
    this.slot = new Float64Array(cap).fill(-1);
    this.sum = new Float64Array(cap);
    this.cnt = new Uint32Array(cap);
    this.min = new Float64Array(cap);
    this.max = new Float64Array(cap);
  }

  add(t: number, v: number): void {
    const s = Math.floor(t / this.res);
    const p = s % this.cap;
    if (this.slot[p] !== s) {
      this.slot[p] = s;
      this.sum[p] = 0;
      this.cnt[p] = 0;
      this.min[p] = Infinity;
      this.max[p] = -Infinity;
    }
    this.sum[p] = (this.sum[p] as number) + v;
    this.cnt[p] = (this.cnt[p] as number) + 1;
    if (v < (this.min[p] as number)) this.min[p] = v;
    if (v > (this.max[p] as number)) this.max[p] = v;
    if (s > this.latest) this.latest = s;
  }

  /** Earliest absolute time (s) still held by the ring. */
  oldestT(): number {
    return this.latest < 0 ? Infinity : Math.max(0, this.latest - this.cap + 1) * this.res;
  }

  accumulate(s: number, acc: Acc): void {
    if (s < 0) return;
    const p = s % this.cap;
    if (this.slot[p] !== s) return;
    const c = this.cnt[p] as number;
    if (!c) return;
    acc.sum += this.sum[p] as number;
    acc.cnt += c;
    if ((this.min[p] as number) < acc.min) acc.min = this.min[p] as number;
    if ((this.max[p] as number) > acc.max) acc.max = this.max[p] as number;
  }

  reset(): void {
    this.slot.fill(-1);
    this.latest = -1;
  }
}

interface Acc { sum: number; cnt: number; min: number; max: number }

/**
 * Multi-resolution time series for one metric: ring buffers at 1 s / 5 s / 30 s / 300 s,
 * each slot keeping sum/count/min/max. Every sample updates all levels (the newest slot of
 * each level is "open" and fills progressively). Memory is fixed (~3.4k slots per metric).
 */
export class MultiResSeries {
  private readonly levels = HISTORY_LEVELS.map((l) => new Level(l.res, l.cap));

  add(t: number, v: number | null): void {
    if (v === null || !Number.isFinite(v) || t < 0) return;
    for (const l of this.levels) l.add(t, v);
  }

  reset(): void {
    for (const l of this.levels) l.reset();
  }

  /** Aggregate the absolute time range [t0, t1) (integer seconds) using level L, recursing to finer levels for ragged edges. */
  private agg(t0: number, t1: number, L: number, acc: Acc): void {
    if (t1 <= t0) return;
    const lv = this.levels[L] as Level;
    if (L === 0) {
      for (let s = t0; s < t1; s++) lv.accumulate(s, acc);
      return;
    }
    const r = lv.res;
    const a = Math.ceil(t0 / r);
    const b = Math.floor(t1 / r);
    const finer = this.levels[L - 1] as Level;
    const coarse = (x0: number, x1: number) => {
      // finer level no longer covers this range: use the coarse slot if its midpoint falls inside
      if (x1 <= x0) return;
      const s = Math.floor(x0 / r);
      const mid = (s + 0.5) * r;
      if (mid >= x0 && mid < x1) lv.accumulate(s, acc);
    };
    const ragged = (x0: number, x1: number) => {
      if (x1 <= x0) return;
      const f0 = finer.oldestT();
      if (x0 >= f0) this.agg(x0, x1, L - 1, acc);
      else if (x1 <= f0) coarse(x0, x1);
      else {
        coarse(x0, f0);
        this.agg(f0, x1, L - 1, acc);
      }
    };
    if (a < b) {
      for (let s = a; s < b; s++) lv.accumulate(s, acc);
      ragged(t0, a * r);
      ragged(b * r, t1);
    } else {
      ragged(t0, t1);
    }
  }

  /**
   * Buckets covering [now - spanS, now], edges uniform on the given time-ago scale (see
   * scales.ts) and snapped to whole seconds; buckets narrower than 1 s are merged, so the
   * result has *about* `buckets` entries. Each bucket uses the coarsest resolution whose
   * slot width fits inside it. Ordered oldest → newest. Buckets without data are omitted
   * (detect gaps via tAgo/dur continuity). tAgo = time-ago of the bucket's newest edge, measured
   * from the end of the current (still filling) second, so it is a whole number and buckets tile
   * exactly: the bucket covers [tAgo, tAgo + dur] seconds ago (±1 s vs the exact ride time).
   */
  query(now: number, spanS: number, buckets: number, scale: TimeScale): Bucket[] {
    const n = Math.max(1, Math.min(2000, Math.round(buckets)));
    const nowEdge = Math.floor(now) + 1;
    const edges = timeAgoEdges(spanS, n, scale);
    const out: Bucket[] = [];
    let hi = nowEdge;
    for (let k = 1; k < edges.length; k++) {
      let lo = Math.max(0, Math.round(nowEdge - (edges[k] as number)));
      // snap to the grid of the finest level that still holds this time, so aggregation is exact
      const cover = this.levels.find((l) => l.oldestT() <= lo);
      if (cover) lo = Math.floor(lo / cover.res) * cover.res;
      if (lo >= hi) continue;
      const width = hi - lo;
      let L = 0;
      for (let i = this.levels.length - 1; i >= 0; i--) {
        if ((this.levels[i] as Level).res <= width) {
          L = i;
          break;
        }
      }
      // if that level's ring no longer reaches back far enough, go coarser
      while (L < this.levels.length - 1 && lo < (this.levels[L] as Level).oldestT()) L++;
      const acc: Acc = { sum: 0, cnt: 0, min: Infinity, max: -Infinity };
      this.agg(lo, hi, L, acc);
      if (acc.cnt > 0) {
        out.push({ tAgo: nowEdge - hi, dur: width, mean: acc.sum / acc.cnt, min: acc.min, max: acc.max });
      }
      hi = lo;
      if (lo <= 0) break;
    }
    return out.reverse();
  }
}

/** One MultiResSeries per metric. */
export class HistoryStore {
  private readonly series = new Map<MetricId, MultiResSeries>();

  add(metric: MetricId, t: number, v: number | null): void {
    let s = this.series.get(metric);
    if (!s) {
      s = new MultiResSeries();
      this.series.set(metric, s);
    }
    s.add(t, v);
  }

  query(metric: MetricId, now: number, spanS: number, buckets: number, scale: TimeScale): Bucket[] {
    return this.series.get(metric)?.query(now, spanS, buckets, scale) ?? [];
  }

  reset(): void {
    this.series.clear();
  }
}
