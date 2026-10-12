import { describe, expect, it } from "vitest";
import type { Sample } from "../src";
import { buildHrRecoveryState, fitMonoExp, predictAt, Rng } from "../src";

const synth = (opts: { hrInf: number; amp: number; tau: number; n: number; t0?: number; noise?: number; seed?: number; round?: boolean }) => {
  const rng = new Rng(opts.seed ?? 7);
  const t0 = opts.t0 ?? 1000;
  const ts: number[] = [];
  const ys: number[] = [];
  for (let i = 0; i < opts.n; i++) {
    const y = opts.hrInf + opts.amp * Math.exp(-i / opts.tau) + rng.normal(0, opts.noise ?? 1);
    ts.push(t0 + i);
    ys.push(opts.round ? Math.round(y) : y);
  }
  return { ts, ys, t0 };
};

const prior = (tauS: number, hrInf = 110) => ({ tauS, tauLogSd: 0.35, hrInf, hrInfSd: 8 });

describe("fitMonoExp (variable projection)", () => {
  for (const tau of [20, 45, 90]) {
    it(`recovers τ = ${tau} s from 3 τ of noisy data within 10 %`, () => {
      const { ts, ys, t0 } = synth({ hrInf: 105, amp: 60, tau, n: Math.round(3 * tau), noise: 1, round: true });
      // deliberately wrong prior: τ_usual = 60 for all cases
      const f = fitMonoExp(ts, ys, t0, prior(60))!;
      expect(Math.abs(f.tauS - tau) / tau).toBeLessThan(0.1);
      expect(f.hrInf).toBeCloseTo(105, -0.5); // ±~1.6 bpm
      expect(f.amp).toBeGreaterThan(55);
      expect(f.amp).toBeLessThan(65);
      expect(f.sigma).toBeGreaterThan(0.7);
      expect(f.sigma).toBeLessThan(1.6);
    });
  }

  it("recovers parameters across many noise seeds (bias check)", () => {
    const taus: number[] = [];
    for (let seed = 1; seed <= 40; seed++) {
      const { ts, ys, t0 } = synth({ hrInf: 108, amp: 55, tau: 50, n: 150, noise: 1.5, seed });
      taus.push(fitMonoExp(ts, ys, t0, prior(70))!.tauS);
    }
    const mean = taus.reduce((a, b) => a + b, 0) / taus.length;
    expect(Math.abs(mean - 50)).toBeLessThan(2.5);
    expect(Math.max(...taus)).toBeLessThan(60);
    expect(Math.min(...taus)).toBeGreaterThan(42);
  });

  it("leans on the usual τ while data is scarce, then lets data win", () => {
    const full = synth({ hrInf: 105, amp: 60, tau: 30, n: 150, noise: 2 });
    // hrInf only loosely known, so τ is not identified by the initial slope alone
    const usual = { ...prior(70), hrInfSd: 30 };
    const flat = { ...usual, tauLogSd: 10 };
    const early = fitMonoExp(full.ts.slice(0, 5), full.ys.slice(0, 5), full.t0, usual)!;
    const earlyFlat = fitMonoExp(full.ts.slice(0, 5), full.ys.slice(0, 5), full.t0, flat)!;
    // the prior pulls the scarce-data estimate towards the usual τ ...
    expect(Math.abs(Math.log(early.tauS / 70))).toBeLessThan(Math.abs(Math.log(earlyFlat.tauS / 70)));
    // ... and is overruled once the curve has bent
    const late = fitMonoExp(full.ts, full.ys, full.t0, usual)!;
    expect(Math.abs(late.tauS - 30)).toBeLessThan(3);
    // posterior over τ is broad early and tight late
    const sdLog = (f: typeof early) => {
      const m = f.grid.reduce((a, g) => a + g.w * Math.log(g.tau), 0);
      return Math.sqrt(f.grid.reduce((a, g) => a + g.w * (Math.log(g.tau) - m) ** 2, 0));
    };
    expect(sdLog(early)).toBeGreaterThan(3 * sdLog(late));
  });

  it("prediction band narrows as data accumulates and covers the truth", () => {
    const truth = (t: number) => 105 + 60 * Math.exp(-(t - 1000) / 45);
    const full = synth({ hrInf: 105, amp: 60, tau: 45, n: 200, noise: 1 });
    const widths: number[] = [];
    for (const n of [10, 30, 60, 120]) {
      const f = fitMonoExp(full.ts.slice(0, n), full.ys.slice(0, n), full.t0, prior(45))!;
      const p = predictAt(f, 1000 + 180);
      widths.push(p.hi - p.lo);
      if (n >= 30) {
        expect(truth(1180)).toBeGreaterThan(p.lo - 1);
        expect(truth(1180)).toBeLessThan(p.hi + 1);
      }
    }
    for (let i = 1; i < widths.length; i++) expect(widths[i]!).toBeLessThan(widths[i - 1]!);
  });

  it("returns null with too few points", () => {
    expect(fitMonoExp([0, 1], [150, 149], 0, prior(50))).toBeNull();
  });
});

describe("buildHrRecoveryState", () => {
  const observed = (hrInf: number, amp: number, tau: number, n: number, startedT = 500, rise = 3, seed = 3): Sample[] => {
    const rng = new Rng(seed);
    const out: Sample[] = [];
    for (let i = 0; i < n; i++) {
      // HR still rises for `rise` seconds after the interval ends (transport delay), then decays
      const y = i < rise ? hrInf + amp - (rise - i) : hrInf + amp * Math.exp(-(i - rise) / tau);
      out.push({ t: startedT + i, v: i === 20 || i === 21 ? null : Math.round(y + rng.normal(0, 0.8)) });
    }
    return out;
  };
  const base = { startedT: 500, minS: 60, maxS: 240, usualTauS: 50, priorHrInf: 108 };

  it("finds the post-interval peak, fits from there and predicts the ready time", () => {
    const obs = observed(105, 65, 45, 70);
    const st = buildHrRecoveryState({ ...base, targetBpm: 120, observed: obs, now: 569 });
    expect(st.active).toBe(true);
    expect(st.peakT).toBeGreaterThanOrEqual(502);
    expect(st.peakT).toBeLessThanOrEqual(504);
    expect(st.fit).toBeDefined();
    expect(Math.abs(st.fit!.tauS - 45)).toBeLessThan(6);
    expect(st.fit!.usualTauS).toBe(50);
    expect(st.unreachable).toBe(false);
    // analytic: crosses 120 at peak + τ ln(65/15) ≈ 503 + 66 = 569 → ~0 s from now
    const tCross = 503 + 45 * Math.log(65 / 15);
    expect(st.readyInS!.mean).toBeCloseTo(Math.max(0, tCross - 569), -1);
    expect(st.readyInS!.lo).toBeLessThanOrEqual(st.readyInS!.mean);
    expect(st.readyInS!.hi).toBeGreaterThanOrEqual(st.readyInS!.mean);
    expect(st.observed).toHaveLength(70);
    expect(st.observed[20]!.v).toBeNull();
  });

  it("forecasts ahead from early data with a band", () => {
    const obs = observed(105, 65, 45, 30);
    const st = buildHrRecoveryState({ ...base, targetBpm: 115, observed: obs, now: 529 });
    const p = st.predicted!;
    expect(p.length).toBeGreaterThan(10);
    expect(p[0]!.t).toBe(529);
    expect(p.at(-1)!.t).toBe(500 + 240);
    for (const x of p) {
      expect(x.lo).toBeLessThanOrEqual(x.mean);
      expect(x.hi).toBeGreaterThanOrEqual(x.mean);
    }
    const tCross = 503 + 45 * Math.log(65 / 10) - 529; // ≈ 58 s
    expect(st.readyInS!.mean).toBeGreaterThan(tCross - 25);
    expect(st.readyInS!.mean).toBeLessThan(tCross + 25);
    expect(st.readyInS!.hi - st.readyInS!.lo).toBeGreaterThan(5);
  });

  it("enforces the min duration even when HR is already below target", () => {
    const obs = observed(95, 40, 15, 40);
    const st = buildHrRecoveryState({ ...base, targetBpm: 120, observed: obs, now: 539 });
    expect(st.readyInS).toEqual({ mean: 21, lo: 21, hi: 21 }); // 500 + 60 − 539
  });

  it("caps at the max duration and flags an unreachable target", () => {
    const obs = observed(126, 40, 30, 120);
    const st = buildHrRecoveryState({ ...base, priorHrInf: 124, targetBpm: 120, observed: obs, now: 619 });
    expect(st.unreachable).toBe(true);
    expect(st.fit!.hrInf).toBeGreaterThan(120);
    expect(st.readyInS!.mean).toBe(500 + 240 - 619);
  });

  it("does not fit while HR is still rising", () => {
    const obs: Sample[] = [0, 1, 2, 3].map((i) => ({ t: 500 + i, v: 160 + i }));
    const st = buildHrRecoveryState({ ...base, targetBpm: 120, observed: obs, now: 503 });
    expect(st.fit).toBeUndefined();
    expect(st.peakT).toBe(503);
  });
});
