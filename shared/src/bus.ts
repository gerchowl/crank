import type { MetricId, Sample } from "./contract";
import { HistoryStore } from "./history";

export type SourceListener = (t: number, v: number | null, reset: boolean) => void;

/**
 * Per-second record of the whole ride for every metric (mean of the samples in that second,
 * NaN = no data). Feeds histogram / heatmap / MMP.
 */
export class RideRecord {
  private series = new Map<MetricId, number[]>();
  private acc = new Map<MetricId, { sec: number; sum: number; n: number }>();

  add(m: MetricId, t: number, v: number | null): void {
    const sec = Math.floor(t);
    let a = this.acc.get(m);
    if (!a) {
      a = { sec, sum: 0, n: 0 };
      this.acc.set(m, a);
    }
    if (sec !== a.sec) {
      this.flush(m, a);
      a.sec = sec;
      a.sum = 0;
      a.n = 0;
    }
    if (v !== null && Number.isFinite(v)) {
      a.sum += v;
      a.n++;
    }
  }

  private flush(m: MetricId, a: { sec: number; sum: number; n: number }): void {
    let s = this.series.get(m);
    if (!s) {
      s = [];
      this.series.set(m, s);
    }
    while (s.length < a.sec) s.push(NaN);
    s[a.sec] = a.n ? a.sum / a.n : NaN;
  }

  /** Completed seconds for a metric (the in-progress second is excluded). */
  get(m: MetricId): readonly number[] {
    return this.series.get(m) ?? [];
  }

  reset(): void {
    this.series.clear();
    this.acc.clear();
  }
}

/** Fan-out of source samples: latest value, history, ride record and subscription listeners. */
export class MetricBus {
  readonly history = new HistoryStore();
  readonly record = new RideRecord();
  private latest = new Map<MetricId, Sample>();
  private listeners = new Map<MetricId, Set<SourceListener>>();

  publish(m: MetricId, t: number, v: number | null): void {
    const val = v === null || !Number.isFinite(v) ? null : v;
    this.latest.set(m, { t, v: val });
    this.history.add(m, t, val);
    this.record.add(m, t, val);
    const ls = this.listeners.get(m);
    if (ls) for (const l of ls) l(t, val, false);
  }

  get(m: MetricId): Sample | undefined {
    return this.latest.get(m);
  }

  value(m: MetricId): number | null {
    return this.latest.get(m)?.v ?? null;
  }

  listen(m: MetricId, l: SourceListener): () => void {
    let s = this.listeners.get(m);
    if (!s) {
      s = new Set();
      this.listeners.set(m, s);
    }
    s.add(l);
    return () => {
      s.delete(l);
    };
  }

  get listenerCount(): number {
    let n = 0;
    for (const s of this.listeners.values()) n += s.size;
    return n;
  }

  /** New ride: clear data and tell listeners to reset their filter state. */
  reset(): void {
    this.history.reset();
    this.record.reset();
    this.latest.clear();
    for (const ls of this.listeners.values()) for (const l of ls) l(0, null, true);
  }
}
