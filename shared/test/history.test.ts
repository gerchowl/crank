import { describe, expect, it } from "vitest";
import { MultiResSeries, Rng, timeAgoToU, uToTimeAgo } from "../src";

/** brute-force aggregation over [lo, hi) seconds */
function brute(xs: number[], lo: number, hi: number) {
  const v = xs.slice(Math.max(0, lo), hi).filter(Number.isFinite);
  return { mean: v.reduce((a, b) => a + b, 0) / v.length, min: Math.min(...v), max: Math.max(...v), n: v.length };
}

describe("MultiResSeries", () => {
  const fill = (n: number, seed = 1, hz = 1) => {
    const rng = new Rng(seed);
    const s = new MultiResSeries();
    const perSec: number[] = [];
    for (let sec = 0; sec < n; sec++) {
      let sum = 0;
      for (let k = 0; k < hz; k++) {
        const v = 200 + 100 * Math.sin(sec / 50) + rng.normal(0, 20);
        s.add(sec + k / hz, v);
        sum += v;
      }
      perSec.push(sum / hz);
    }
    return { s, perSec };
  };

  for (const scale of ["log", "fisheye"] as const) {
    it(`buckets tile the span exactly and match brute force (${scale})`, () => {
      const { s, perSec } = fill(1800);
      const now = 1799.75;
      const bs = s.query(now, 1800, 100, scale);
      expect(bs.length).toBeGreaterThan(40);
      expect(bs.length).toBeLessThanOrEqual(100);
      // contiguous, oldest first, total coverage
      for (let i = 1; i < bs.length; i++) expect(bs[i - 1]!.tAgo).toBeCloseTo(bs[i]!.tAgo + bs[i]!.dur);
      expect(bs.at(-1)!.tAgo).toBe(0);
      expect(bs.reduce((a, b) => a + b.dur, 0)).toBe(1800);
      // each bucket's mean/min/max equals brute force over the same seconds (1 s level covers 20 min,
      // older buckets come from coarse slots plus finer ragged edges — still exact here)
      const nowEdge = Math.floor(now) + 1;
      for (const b of bs) {
        const t1 = nowEdge - b.tAgo;
        const t0 = t1 - b.dur;
        const ref = brute(perSec, t0, t1);
        expect(b.mean).toBeCloseTo(ref.mean, 6);
        if (t0 >= nowEdge - 1200) {
          // still inside the 1 s ring: envelope is exact too
          expect(b.min).toBeCloseTo(ref.min, 6);
          expect(b.max).toBeCloseTo(ref.max, 6);
        }
        expect(b.min).toBeLessThanOrEqual(b.mean);
        expect(b.max).toBeGreaterThanOrEqual(b.mean);
      }
    });
  }

  it("keeps sub-second min/max in the envelope", () => {
    const s = new MultiResSeries();
    for (let i = 0; i < 40; i++) s.add(i * 0.25, i % 4 === 0 ? 1000 : 100);
    const bs = s.query(9.9, 10, 10, "log");
    const last = bs.at(-1)!;
    expect(last.max).toBe(1000);
    expect(last.min).toBe(100);
    expect(last.mean).toBeCloseTo(325);
  });

  it("serves a 2 h span from coarse levels after the 1 s ring wrapped", () => {
    const { s, perSec } = fill(7200, 2);
    const bs = s.query(7199.5, 7200, 120, "log");
    expect(bs.reduce((a, b) => a + b.dur, 0)).toBe(7200);
    const oldest = bs[0]!;
    const ref = brute(perSec, 0, oldest.dur);
    expect(oldest.mean).toBeCloseTo(ref.mean, 6);
    // newest buckets are 1 s wide
    expect(bs.at(-1)!.dur).toBe(1);
    expect(bs.at(-2)!.dur).toBe(1);
  });

  it("omits empty buckets (gaps) and buckets before the ride start", () => {
    const s = new MultiResSeries();
    for (let t = 0; t < 100; t++) if (t < 40 || t >= 60) s.add(t, 1);
    const bs = s.query(99.5, 3600, 200, "log");
    const covered = bs.reduce((a, b) => a + b.dur, 0);
    expect(covered).toBeLessThanOrEqual(100);
    expect(covered).toBeGreaterThanOrEqual(75);
    for (const b of bs) expect(b.mean).toBe(1);
  });

  it("scale helpers are inverse and monotone", () => {
    for (const sc of ["log", "fisheye"] as const) {
      let prev = -1;
      for (let u = 0; u <= 1.00001; u += 0.05) {
        const t = uToTimeAgo(u, 7200, sc);
        expect(timeAgoToU(t, 7200, sc)).toBeCloseTo(Math.min(1, u), 9);
        expect(t).toBeGreaterThan(prev);
        prev = t;
      }
    }
  });
});
