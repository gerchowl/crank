// Reactive primitives over the backend. Each one owns its subscription/timer and tears it down
// with the owning component (onCleanup) — that is what makes "telemetry only updates the
// subscribing widget" structural: only the computations that read the returned signal re-run.
import { type Accessor, createEffect, createSignal, onCleanup, untrack } from "solid-js";
import type { Filter, MetricId, Sample } from "@neo/shared";
import { backend } from "../backend";

export interface SubSpec {
  metric: MetricId;
  rateHz: number;
  filter?: Filter[] | undefined;
}

/** Live sample of a metric at a widget's display rate. Re-subscribes when the spec changes. */
export function createSample(spec: Accessor<SubSpec | null>, onSample?: (s: Sample) => void): Accessor<Sample | null> {
  const [sample, setSample] = createSignal<Sample | null>(null, { equals: false });
  createEffect(() => {
    const s = spec();
    if (!s) {
      setSample(null);
      return;
    }
    const opts = s.filter && s.filter.length ? { rateHz: s.rateHz, filter: s.filter } : { rateHz: s.rateHz };
    const off = backend.subscribe(s.metric, opts, (v) => {
      setSample(v);
      onSample?.(v);
    });
    onCleanup(off);
  });
  return sample;
}

/**
 * Poll a (sync) backend query at `hz` while `enabled`. The fetcher runs untracked; the
 * result signal never compares equal so consumers redraw on every poll.
 */
export function createPoll<T>(fetch: () => T, hz: Accessor<number>, enabled: Accessor<boolean> = () => true): Accessor<T | undefined> {
  const [value, setValue] = createSignal<T | undefined>(undefined, { equals: false });
  createEffect(() => {
    const rate = Math.min(hz(), 2);
    if (!enabled()) return;
    const tick = () => setValue(() => untrack(fetch));
    tick();
    const id = window.setInterval(tick, 1000 / Math.max(rate, 0.05));
    onCleanup(() => window.clearInterval(id));
  });
  return value;
}

export interface Size {
  width: number;
  height: number;
}

/** Content-box size of an element, tracked with a ResizeObserver. */
export function createElementSize(el: Accessor<HTMLElement | undefined>): Accessor<Size> {
  const [size, setSize] = createSignal<Size>({ width: 0, height: 0 }, { equals: (a, b) => a.width === b.width && a.height === b.height });
  createEffect(() => {
    const node = el();
    if (!node) return;
    const ro = new ResizeObserver((entries) => {
      const r = entries[0]?.contentRect;
      if (r) setSize({ width: Math.floor(r.width), height: Math.floor(r.height) });
    });
    ro.observe(node);
    onCleanup(() => ro.disconnect());
  });
  return size;
}

/** A window keydown listener bound to the owner's lifetime. */
export function createKeydown(handler: (e: KeyboardEvent) => void): void {
  window.addEventListener("keydown", handler);
  onCleanup(() => window.removeEventListener("keydown", handler));
}
