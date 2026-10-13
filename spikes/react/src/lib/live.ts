// Live-data hooks. Telemetry never goes through React state of a parent: each widget subscribes
// on its own, either into a per-subscription external store (leaf re-render only) or imperatively
// (charts/canvas, no React render at all).
import { useEffect, useEffectEvent, useMemo, useSyncExternalStore } from "react";
import type { CoachNudge, Filter, MetricId, RideStatus, Sample } from "@neo/shared";
import { backend } from "../backend";
import { createStore } from "./store";

interface MetricSource { subscribe: (notify: () => void) => () => void; get: () => Sample | null }

function metricSource(metric: MetricId | undefined, rateHz: number, filter: Filter[] | undefined): MetricSource {
  let last: Sample | null = null;
  return {
    subscribe: (notify) => {
      if (!metric) return () => undefined;
      return backend.subscribe(metric, filter ? { rateHz, filter } : { rateHz }, (s) => {
        last = s;
        notify();
      });
    },
    get: () => last,
  };
}

/** Latest filtered sample of one metric at `rateHz`; re-renders only the calling component. */
export function useMetric(metric: MetricId | undefined, rateHz: number, filter?: Filter[]): Sample | null {
  const src = useMemo(() => metricSource(metric, rateHz, filter), [metric, rateHz, filter]);
  return useSyncExternalStore(src.subscribe, src.get);
}

/** Imperative subscription: `onSample` runs outside React rendering (write to a canvas/uPlot/DOM ref). */
export function useMetricCallback(metric: MetricId | undefined, rateHz: number, filter: Filter[] | undefined, onSample: (s: Sample) => void): void {
  const handler = useEffectEvent(onSample);
  useEffect(() => {
    if (!metric) return;
    return backend.subscribe(metric, filter ? { rateHz, filter } : { rateHz }, (s) => {
      handler(s);
    });
  }, [metric, rateHz, filter]);
}

/** Poll a backend query at `hz` (clamped to `maxHz`, per the polling budget). Fires once immediately. */
export function usePoll(hz: number, maxHz: number, tick: () => void, key: unknown = null): void {
  const run = useEffectEvent(tick);
  const ms = 1000 / Math.max(0.05, Math.min(hz, maxHz));
  useEffect(() => {
    run();
    const id = setInterval(() => {
      run();
    }, ms);
    return () => {
      clearInterval(id);
    };
  }, [ms, key]);
}

// ---------- ride status: one backend listener, many selective readers ----------
const statusStore = createStore<RideStatus | null>(null);
backend.onStatus((s) => {
  statusStore.set(s);
});

export const getStatus = (): RideStatus | null => statusStore.get();

/** Select a slice of the ride status; the component re-renders only when the slice changes. */
export function useRideStatus<T>(select: (s: RideStatus | null) => T): T {
  return useSyncExternalStore(statusStore.subscribe, () => select(statusStore.get()));
}
export const subscribeStatus = statusStore.subscribe;

// ---------- coach nudges ----------
export const nudgeStore = createStore<CoachNudge[]>([]);
backend.onNudge((n) => {
  nudgeStore.set((prev) => {
    const i = prev.findIndex((p) => p.callId === n.callId);
    if (i < 0) return [...prev, n];
    const next = prev.slice();
    next[i] = n;
    return next;
  });
});
export function useNudges(): CoachNudge[] {
  return useSyncExternalStore(nudgeStore.subscribe, nudgeStore.get);
}

// ---------- compiled workout of the ride (changes on load / coach edits, not per frame) ----------
type Current = ReturnType<typeof backend.ride.current>;
const currentStore = createStore<Current>(backend.ride.current());
let lastKey = "";
const refreshCurrent = () => {
  const s = statusStore.get();
  const n = nudgeStore.get();
  const key = `${s?.state ?? ""}|${String(s?.blockIndex ?? -1)}|${String(s?.manualOffsetPct ?? 0)}|${n.map((x) => x.status).join()}`;
  if (key === lastKey) return;
  lastKey = key;
  currentStore.set(backend.ride.current());
};
statusStore.subscribe(refreshCurrent);
nudgeStore.subscribe(refreshCurrent);
export const forceRefreshCurrent = () => {
  lastKey = "";
  refreshCurrent();
};
export function useRideWorkout(): Current {
  return useSyncExternalStore(currentStore.subscribe, currentStore.get);
}
