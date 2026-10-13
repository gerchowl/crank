// Full-ride 1 Hz record for the review, fetched incrementally via backend.streams(metrics, fromS)
// at ≤ 1 Hz and kept in an external store (only views that read it re-render, and only on new seconds).
import { useEffect, useSyncExternalStore } from "react";
import type { MetricId } from "@neo/shared";
import { backend } from "../backend";
import { createStore } from "../lib/store";

export const RECORD_METRICS: MetricId[] = ["power", "hr", "cadence", "dfa_a1", "wbal_pct", "kj", "target_power", "block_index"];

export interface RideRecord { toS: number; series: Record<string, (number | null)[]> }

const store = createStore<RideRecord>({ toS: 0, series: {} });

function pull() {
  const cur = store.get();
  const r = backend.streams(RECORD_METRICS, cur.toS);
  if (r.toS < cur.toS) {
    // workout reloaded: start over
    store.set({ toS: 0, series: {} });
    pull();
    return;
  }
  if (r.toS === cur.toS) return;
  const series: Record<string, (number | null)[]> = {};
  for (const m of RECORD_METRICS) series[m] = [...(cur.series[m] ?? []), ...(r.series[m] ?? [])];
  store.set({ toS: r.toS, series });
}

export function useRideRecord(): RideRecord {
  useEffect(() => {
    pull();
    const id = setInterval(pull, 1000);
    return () => {
      clearInterval(id);
    };
  }, []);
  return useSyncExternalStore(store.subscribe, store.get);
}

export function mean(xs: readonly (number | null | undefined)[]): number | null {
  let s = 0;
  let n = 0;
  for (const v of xs) {
    if (v === null || v === undefined || !Number.isFinite(v)) continue;
    s += v;
    n++;
  }
  return n ? s / n : null;
}
