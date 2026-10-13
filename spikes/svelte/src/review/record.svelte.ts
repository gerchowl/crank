// Incremental full-ride record from backend.streams() (≤ 1 Hz, fetching only new seconds).
// The arrays are plain (non-reactive) and grow in place; `version` is the reactive signal
// charts depend on, so a poll costs one integer bump instead of proxying 10k-element arrays.
import type { MetricId } from "@neo/shared";
import { backend } from "../lib/backend";

export const STREAM_METRICS: MetricId[] = ["power", "hr", "cadence", "dfa_a1", "wbal_pct", "kj", "target_power", "block_index"];

export class RideRecord {
  t: number[] = [];
  // eslint-disable-next-line svelte/prefer-svelte-reactivity -- deliberately non-reactive; `version` is the signal
  series = new Map<MetricId, (number | null)[]>(STREAM_METRICS.map((m) => [m, []]));
  version = $state(0);
  #toS = 0;

  /** Read inside an effect/derived to depend on new data arriving. */
  track(): number {
    return this.version;
  }

  get(m: MetricId): (number | null)[] {
    return this.series.get(m) ?? [];
  }

  /** Fetch new seconds. Restarts from 0 if the ride was reset. */
  pull(): void {
    let r = backend.streams(STREAM_METRICS, this.#toS);
    if (r.toS < this.#toS) {
      this.reset();
      r = backend.streams(STREAM_METRICS);
    }
    const n = r.toS - r.fromS;
    if (n <= 0) return;
    for (let i = 0; i < n; i++) this.t.push(r.fromS + i);
    for (const m of STREAM_METRICS) {
      const dst = this.series.get(m);
      const src = r.series[m];
      if (!dst) continue;
      for (let i = 0; i < n; i++) dst.push(src?.[i] ?? null);
    }
    this.#toS = r.toS;
    this.version++;
  }

  reset(): void {
    this.t = [];
    for (const m of STREAM_METRICS) this.series.set(m, []);
    this.#toS = 0;
    this.version++;
  }
}
