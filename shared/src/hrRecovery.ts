import type { HrRecoveryState, Sample } from "./contract";

/**
 * Live HR-recovery fit.
 *
 * Model (from the post-interval HR peak at t0):  hr(t) = hrInf + amp · exp(-(t - t0) / τ)
 *
 * Variable projection: for every τ on a log-spaced grid (10–200 s) the model is linear in
 * (hrInf, amp), so those are solved exactly by 2×2 (regularised) least squares. Over τ we
 * keep the whole profile rather than just the minimum:
 *
 *   J(τ) = SSE(τ)/σ² + (hrInf − hrInf₀)²/s_a² + (amp − amp₀)²/s_b² + ((ln τ − ln τ_usual)/s_τ)² + ln det A(τ)
 *
 * i.e. a Gaussian prior on hrInf (expected steady HR at the recovery power), a weak one on amp
 * (first sample − hrInf₀, sd 20 bpm), a log-normal prior on τ centred on the athlete's usual τ,
 * and the Laplace-marginalised (hrInf, amp) evidence term ln det A. Posterior weights w(τ) ∝ exp(−J/2). While data is scarce the prior dominates (the
 * early part of an exponential is nearly a straight line: slope = amp/τ is identified, τ is
 * not); as the curve bends the data takes over. This is the "blend" of usual τ and data.
 *
 * Point estimate: MAP τ (parabolic refinement in ln τ). Prediction band: mixture over the τ
 * grid of the per-τ linear-Gaussian predictive (parameter uncertainty only, 90 % band).
 * σ is estimated from residuals once there are ≥ 12 points (clamped to 0.8–6 bpm).
 */

export interface RecoveryPrior {
  tauS: number;
  /** sd of ln τ (0.35 ≈ ±40 %) */
  tauLogSd: number;
  hrInf: number;
  hrInfSd: number;
}

export interface TauGridPoint {
  tau: number;
  w: number;     // normalised posterior weight
  a: number;     // hrInf
  b: number;     // amp
  c00: number; c01: number; c11: number; // posterior covariance of (a, b)
}

export interface MonoExpFit {
  hrInf: number;
  amp: number;
  tauS: number;
  sigma: number;
  n: number;
  t0: number;
  grid: TauGridPoint[];
}

export interface FitOptions {
  tauMin?: number;
  tauMax?: number;
  gridN?: number;
  /** fix the noise sd instead of estimating it */
  sigma?: number;
}

interface Sums { n: number; sy: number; syy: number }

/** Weak Gaussian prior on the amplitude (centred on first sample − prior hrInf) keeps the marginal proper. */
const AMP_PRIOR_SD = 20;

function solveAt(tau: number, ts: readonly number[], ys: readonly number[], t0: number, s: Sums, sigma: number, prior: RecoveryPrior, amp0: number) {
  let sx = 0, sxx = 0, sxy = 0;
  for (let i = 0; i < ts.length; i++) {
    const x = Math.exp(-((ts[i] as number) - t0) / tau);
    sx += x;
    sxx += x * x;
    sxy += x * (ys[i] as number);
  }
  const is2 = 1 / (sigma * sigma);
  const ia2 = 1 / (prior.hrInfSd * prior.hrInfSd);
  const ib2 = 1 / (AMP_PRIOR_SD * AMP_PRIOR_SD);
  const A00 = s.n * is2 + ia2;
  const A01 = sx * is2;
  const A11 = sxx * is2 + ib2;
  const r0 = s.sy * is2 + prior.hrInf * ia2;
  const r1 = sxy * is2 + amp0 * ib2;
  const det = A00 * A11 - A01 * A01;
  const a = (r0 * A11 - A01 * r1) / det;
  const b = (A00 * r1 - A01 * r0) / det;
  const sse = Math.max(0, s.syy - 2 * a * s.sy - 2 * b * sxy + s.n * a * a + 2 * a * b * sx + b * b * sxx);
  const lt = (Math.log(tau) - Math.log(prior.tauS)) / prior.tauLogSd;
  const J = sse * is2 + (a - prior.hrInf) ** 2 * ia2 + (b - amp0) ** 2 * ib2 + lt * lt + Math.log(det);
  return { a, b, sse, J, c00: A11 / det, c01: -A01 / det, c11: A00 / det };
}

export function fitMonoExp(
  ts: readonly number[],
  ys: readonly number[],
  t0: number,
  prior: RecoveryPrior,
  opts: FitOptions = {},
): MonoExpFit | null {
  const n = ts.length;
  if (n < 3 || ys.length !== n) return null;
  const tauMin = opts.tauMin ?? 10;
  const tauMax = opts.tauMax ?? 200;
  const G = opts.gridN ?? 96;
  let sy = 0, syy = 0;
  for (const y of ys) {
    sy += y;
    syy += y * y;
  }
  const sums: Sums = { n, sy, syy };
  const amp0 = Math.max(0, (ys[0] as number) - prior.hrInf);
  const taus: number[] = [];
  for (let j = 0; j < G; j++) taus.push(tauMin * Math.pow(tauMax / tauMin, j / (G - 1)));

  const run = (sigma: number) => {
    const sol = taus.map((tau) => solveAt(tau, ts, ys, t0, sums, sigma, prior, amp0));
    let jmin = 0;
    for (let j = 1; j < G; j++) if ((sol[j] as { J: number }).J < (sol[jmin] as { J: number }).J) jmin = j;
    return { sol, jmin };
  };

  let sigma = opts.sigma ?? 1.5;
  let { sol, jmin } = run(sigma);
  if (opts.sigma === undefined && n >= 12) {
    const est = Math.sqrt((sol[jmin] as { sse: number }).sse / (n - 2));
    sigma = Math.min(6, Math.max(0.8, est));
    ({ sol, jmin } = run(sigma));
  }

  // posterior weights
  const Jmin = (sol[jmin] as { J: number }).J;
  let wsum = 0;
  const ws = sol.map((s) => {
    const w = Math.exp(-(s.J - Jmin) / 2);
    wsum += w;
    return w;
  });
  const grid: TauGridPoint[] = [];
  for (let j = 0; j < G; j++) {
    const w = (ws[j] as number) / wsum;
    if (w < 1e-6) continue;
    const s = sol[j] as ReturnType<typeof solveAt>;
    grid.push({ tau: taus[j] as number, w, a: s.a, b: s.b, c00: s.c00, c01: s.c01, c11: s.c11 });
  }

  // MAP τ with parabolic refinement in ln τ
  let tauHat = taus[jmin] as number;
  if (jmin > 0 && jmin < G - 1) {
    const y0 = (sol[jmin - 1] as { J: number }).J;
    const y1 = (sol[jmin] as { J: number }).J;
    const y2 = (sol[jmin + 1] as { J: number }).J;
    const den = y0 - 2 * y1 + y2;
    if (den > 0) {
      const off = (0.5 * (y0 - y2)) / den; // in grid steps, |off| ≤ 0.5
      const step = Math.log(tauMax / tauMin) / (G - 1);
      tauHat = Math.exp(Math.log(tauHat) + Math.max(-0.5, Math.min(0.5, off)) * step);
    }
  }
  const best = solveAt(tauHat, ts, ys, t0, sums, sigma, prior, amp0);
  return { hrInf: best.a, amp: best.b, tauS: tauHat, sigma, n, t0, grid };
}

/** Mixture predictive at absolute time t: mean and 90 % band (curve uncertainty, not sample noise). */
export function predictAt(fit: MonoExpFit, t: number): { mean: number; lo: number; hi: number } {
  let m = 0;
  let m2 = 0;
  for (const g of fit.grid) {
    const x = Math.exp(-(t - fit.t0) / g.tau);
    const mu = g.a + g.b * x;
    const v = Math.max(0, g.c00 + 2 * g.c01 * x + g.c11 * x * x);
    m += g.w * mu;
    m2 += g.w * (v + mu * mu);
  }
  const sd = Math.sqrt(Math.max(0, m2 - m * m));
  return { mean: m, lo: m - 1.645 * sd, hi: m + 1.645 * sd };
}

export interface RecoveryInput {
  startedT: number;
  minS: number;
  maxS: number;
  targetBpm: number;
  /** HR samples since the block started (1 Hz, null = dropout) */
  observed: readonly Sample[];
  /** current ride time */
  now: number;
  usualTauS: number;
  /** expected steady HR at the recovery power (prior mean for hrInf) */
  priorHrInf: number;
  priorHrInfSd?: number;
  tauLogSd?: number;
  /** how long after block start to look for the post-interval peak (s) */
  peakSearchS?: number;
}

/** Build the contract's HrRecoveryState from raw samples. Pure; safe to call at a few Hz. */
export function buildHrRecoveryState(inp: RecoveryInput): HrRecoveryState {
  const peakSearch = inp.peakSearchS ?? 60;
  const valid = inp.observed.filter((s): s is { t: number; v: number } => s.v !== null);
  let peakIdx = -1;
  for (let i = 0; i < valid.length; i++) {
    const s = valid[i] as { t: number; v: number };
    if (s.t > inp.startedT + peakSearch) break;
    if (peakIdx < 0 || s.v >= (valid[peakIdx] as { v: number }).v) peakIdx = i;
  }
  const peakT = peakIdx >= 0 ? (valid[peakIdx] as { t: number }).t : inp.startedT;
  const base: HrRecoveryState = {
    active: true,
    startedT: inp.startedT,
    peakT,
    targetBpm: inp.targetBpm,
    observed: inp.observed.map((s) => ({ t: s.t, v: s.v })),
    unreachable: false,
    minS: inp.minS,
    maxS: inp.maxS,
  };
  if (peakIdx < 0) return base;
  const pts = valid.slice(peakIdx);
  if (pts.length < 4) return base;

  const prior: RecoveryPrior = {
    tauS: inp.usualTauS,
    tauLogSd: inp.tauLogSd ?? 0.35,
    hrInf: inp.priorHrInf,
    hrInfSd: inp.priorHrInfSd ?? 8,
  };
  const fit = fitMonoExp(pts.map((p) => p.t), pts.map((p) => p.v), peakT, prior);
  if (!fit) return base;

  const endT = inp.startedT + inp.maxS;
  const now = Math.max(inp.now, (pts[pts.length - 1] as { t: number }).t);
  const earliest = Math.max(0, inp.startedT + inp.minS - now);
  const latest = Math.max(0, endT - now);

  // crossing times on a 1 s grid up to the max duration
  let cMean: number | null = null;
  let cLo: number | null = null;
  let cHi: number | null = null;
  const predicted: { t: number; mean: number; lo: number; hi: number }[] = [];
  const horizon = Math.max(0, endT - now);
  const step = Math.max(1, Math.ceil(horizon / 90));
  for (let dt = 0; dt <= horizon; dt++) {
    const p = predictAt(fit, now + dt);
    if (cLo === null && p.lo <= inp.targetBpm) cLo = dt;
    if (cMean === null && p.mean <= inp.targetBpm) cMean = dt;
    if (cHi === null && p.hi <= inp.targetBpm) cHi = dt;
    if (dt % step === 0 || dt === horizon) predicted.push({ t: now + dt, mean: p.mean, lo: p.lo, hi: p.hi });
  }
  const clampR = (x: number | null) => Math.min(latest, Math.max(earliest, x ?? latest));
  const lastHr = (pts[pts.length - 1] as { v: number }).v;
  const already = lastHr <= inp.targetBpm;
  const readyInS = already
    ? { mean: earliest, lo: earliest, hi: earliest }
    : { mean: clampR(cMean), lo: clampR(cLo), hi: clampR(cHi) };

  return {
    ...base,
    fit: { hrInf: fit.hrInf, amp: fit.amp, tauS: fit.tauS, usualTauS: inp.usualTauS },
    predicted,
    readyInS,
    unreachable: fit.hrInf >= inp.targetBpm,
  };
}

export function emptyRecoveryState(): HrRecoveryState {
  return { active: false, startedT: 0, peakT: 0, targetBpm: 0, observed: [], unreachable: false, minS: 0, maxS: 0 };
}
