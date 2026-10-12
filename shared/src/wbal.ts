/**
 * W' balance, Skiba "differential" model (Skiba et al. 2015; Froncioni/Clarke form):
 *   P > CP:  dW'/dt = -(P - CP)                       (linear depletion)
 *   P ≤ CP:  dW'/dt = (W'0 - W'bal) · (CP - P) / W'0  (exponential refill)
 * The refill part is integrated exactly over dt so the result is step-size independent
 * for piecewise-constant power. W'bal may go negative (= target not sustainable).
 */
export function wbalStep(wbal: number, powerW: number, dt: number, cp: number, w0: number): number {
  if (powerW > cp) return wbal - (powerW - cp) * dt;
  return w0 - (w0 - wbal) * Math.exp((-(cp - powerW) * dt) / w0);
}

/** W'bal trace for a power series sampled every dt seconds. Starts full. */
export function wbalSeries(power: ArrayLike<number>, dt: number, cp: number, w0: number): Float64Array {
  const out = new Float64Array(power.length);
  let w = w0;
  for (let i = 0; i < power.length; i++) {
    w = wbalStep(w, power[i] as number, dt, cp, w0);
    out[i] = w;
  }
  return out;
}
