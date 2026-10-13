// Lifecycle-bound data hooks. Call these during component init: the $effect inside ties the
// subscription/timer to the component (and re-creates it when the reactive inputs it reads
// change — e.g. a widget's rateHz or filter in edit mode).
import type { MetricId, Sample, SubscribeOpts } from "@neo/shared";
import { backend } from "./backend";

/**
 * Subscribe to one metric. Returns a reactive `sample` (only this component re-renders) and
 * optionally forwards every sample to `onSample` for imperative chart updates.
 */
export function liveMetric(
  metric: () => MetricId | undefined,
  opts: () => SubscribeOpts,
  onSample?: (s: Sample) => void,
): { readonly sample: Sample | null; readonly v: number | null } {
  let sample = $state.raw<Sample | null>(null);
  $effect(() => {
    const m = metric();
    if (!m) return;
    // Snapshot so the backend never holds a reactive proxy of the widget config.
    const o = $state.snapshot(opts());
    const off = backend.subscribe(m, o, (s) => {
      sample = s;
      onSample?.(s);
    });
    return () => {
      off();
      sample = null;
    };
  });
  return {
    get sample() {
      return sample;
    },
    get v() {
      return sample?.v ?? null;
    },
  };
}

/**
 * Poll a backend query at `hz` (wall clock) while `active()` is true, plus once immediately.
 * The query itself is called untracked (inside a timer), so polling never subscribes to
 * reactive state by accident.
 */
export function poll<T>(query: () => T, hz: () => number, active: () => boolean = () => true): { readonly value: T | undefined } {
  let value = $state.raw<T | undefined>(undefined);
  $effect(() => {
    const ms = 1000 / Math.max(0.05, Math.min(2, hz()));
    const on = active();
    value = query();
    if (!on) return;
    const id = setInterval(() => {
      value = query();
    }, ms);
    return () => {
      clearInterval(id);
    };
  });
  return {
    get value() {
      return value;
    },
  };
}
