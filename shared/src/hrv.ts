/**
 * HRV features from RR intervals (ms).
 *
 * rmssd: root mean square of successive differences over the given beats, skipping
 * artefacts (successive change > 20 %).
 *
 * dfa_a1: short-term scaling exponent of detrended fluctuation analysis (Peng 1995) over a
 * trailing window of beats (the backend uses 120 beats, recomputed at 1 Hz), box sizes
 * n = 4..16, non-overlapping boxes, linear detrending, slope of log F(n) vs log n.
 * Before DFA the series is detrended with a centred 25-beat moving average (a cheap stand-in for
 * the smoothness-priors detrending Kubios uses) so workload-driven HR changes don't inflate α1.
 * Otherwise this is the standard algorithm on a short window; the caveat is in the *input*: the mock's
 * RR series is synthetic (an intensity-dependent mix of AR(1), white and anti-correlated
 * noise tuned so α1 ≈ 1.0–1.2 at easy intensity, ≈ 0.75 near VT1, ≈ 0.5 near threshold and
 * < 0.5 above). It is a plausible stand-in, not a physiological model.
 */
export function rmssd(rr: readonly number[]): number | null {
  let s = 0;
  let n = 0;
  for (let i = 1; i < rr.length; i++) {
    const a = rr[i - 1] as number;
    const b = rr[i] as number;
    const d = b - a;
    if (Math.abs(d) > 0.2 * a) continue;
    s += d * d;
    n++;
  }
  return n >= 5 ? Math.sqrt(s / n) : null;
}

/** Remove slow trends (HR changing with workload) with a centred moving average (window w beats). */
export function detrendRr(rr: readonly number[], w = 41): number[] {
  const N = rr.length;
  const h = w >> 1;
  const pre = new Float64Array(N + 1);
  for (let i = 0; i < N; i++) pre[i + 1] = (pre[i] as number) + (rr[i] as number);
  const out = new Array<number>(N);
  for (let i = 0; i < N; i++) {
    const a = Math.max(0, i - h);
    const b = Math.min(N, i + h + 1);
    out[i] = (rr[i] as number) - ((pre[b] as number) - (pre[a] as number)) / (b - a);
  }
  return out;
}

export function dfaAlpha1(rrRaw: readonly number[], minBox = 4, maxBox = 16, detrendWindow = 25): number | null {
  const N = rrRaw.length;
  if (N < maxBox * 3) return null;
  const rr = detrendWindow > 0 ? detrendRr(rrRaw, detrendWindow) : rrRaw;
  let m = 0;
  for (const x of rr) m += x;
  m /= N;
  const y = new Float64Array(N);
  let c = 0;
  for (let i = 0; i < N; i++) {
    c += (rr[i] as number) - m;
    y[i] = c;
  }
  const lx: number[] = [];
  const ly: number[] = [];
  for (let n = minBox; n <= maxBox; n++) {
    const boxes = Math.floor(N / n);
    if (boxes < 2) break;
    // x = 0..n-1, precompute sums for linear regression
    const sx = (n * (n - 1)) / 2;
    const sxx = ((n - 1) * n * (2 * n - 1)) / 6;
    const den = n * sxx - sx * sx;
    let f2 = 0;
    for (let b = 0; b < boxes; b++) {
      const o = b * n;
      let sy = 0;
      let sxy = 0;
      for (let i = 0; i < n; i++) {
        const v = y[o + i] as number;
        sy += v;
        sxy += i * v;
      }
      const slope = (n * sxy - sx * sy) / den;
      const icpt = (sy - slope * sx) / n;
      for (let i = 0; i < n; i++) {
        const r = (y[o + i] as number) - (icpt + slope * i);
        f2 += r * r;
      }
    }
    const F = Math.sqrt(f2 / (boxes * n));
    if (F > 0) {
      lx.push(Math.log(n));
      ly.push(Math.log(F));
    }
  }
  if (lx.length < 3) return null;
  const k = lx.length;
  let ax = 0, ay = 0;
  for (let i = 0; i < k; i++) {
    ax += lx[i] as number;
    ay += ly[i] as number;
  }
  ax /= k;
  ay /= k;
  let num = 0, den = 0;
  for (let i = 0; i < k; i++) {
    num += ((lx[i] as number) - ax) * ((ly[i] as number) - ay);
    den += ((lx[i] as number) - ax) ** 2;
  }
  return den > 0 ? num / den : null;
}
