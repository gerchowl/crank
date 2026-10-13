import { type Accessor, createSignal, onCleanup } from "solid-js";
import type { HrRecoveryState, MetricId } from "@neo/shared";
import { backend } from "../backend";
import { ride } from "../state/ride";

export const REVIEW_METRICS: MetricId[] = ["power", "hr", "cadence", "dfa_a1", "wbal_pct", "kj", "np", "if", "tss", "decoupling", "compliance"];

export interface RideRecord {
  len: number;
  series: Partial<Record<MetricId, (number | null)[]>>;
  recoveries: HrRecoveryState[];
}

/**
 * Full-ride 1 Hz record via backend.streams(), fetched incrementally (fromS = previous toS)
 * at ≤ 1 Hz while the ride is moving. Arrays are appended in place; the signal is bumped.
 */
export function createRideRecord(): Accessor<RideRecord> {
  const rec: RideRecord = { len: 0, series: {}, recoveries: [] };
  const [get, set] = createSignal(rec, { equals: false });
  const pull = () => {
    if (rec.len > 0 && ride.status.t + 2 < rec.len) {
      rec.len = 0; // a new ride was loaded
      rec.series = {};
    }
    const s = backend.streams(REVIEW_METRICS, rec.len);
    for (const m of REVIEW_METRICS) {
      const add = s.series[m] ?? [];
      const cur = rec.series[m] ?? (rec.series[m] = []);
      cur.length = s.fromS;
      for (const v of add) cur.push(v);
    }
    rec.len = s.toS;
    rec.recoveries = backend.recoveries();
    set(rec);
  };
  pull();
  const id = window.setInterval(() => {
    if (ride.status.state === "riding") pull();
  }, 1000);
  onCleanup(() => window.clearInterval(id));
  return get;
}

export function lastValue(xs: readonly (number | null)[] | undefined): number | null {
  if (!xs) return null;
  for (let i = xs.length - 1; i >= 0; i--) {
    const v = xs[i];
    if (v != null) return v;
  }
  return null;
}

export function mean(xs: readonly (number | null)[] | undefined, from = 0, to = xs?.length ?? 0): number | null {
  if (!xs) return null;
  let s = 0;
  let n = 0;
  for (let i = Math.max(0, Math.floor(from)); i < Math.min(xs.length, Math.ceil(to)); i++) {
    const v = xs[i];
    if (v != null) {
      s += v;
      n++;
    }
  }
  return n ? s / n : null;
}
