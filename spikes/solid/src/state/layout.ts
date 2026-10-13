// Ride layout as a store: widgets are store proxies, so moving/resizing/re-configuring one
// widget only re-runs the computations that read that widget's changed fields.
import { batch, createEffect, createRoot, createSignal, on } from "solid-js";
import { createStore, produce, reconcile, unwrap } from "solid-js/store";
import type { Layout, MetricId, Page, Widget, WidgetKind } from "@neo/shared";
import { backend } from "../backend";
import { ride } from "./ride";

export const GRID_COLS = 12;

/** Patch where `undefined` means "remove the optional field". */
export type WidgetPatch = { [K in keyof Widget]?: Widget[K] | undefined };

function defaultDevice(): Layout["device"] {
  return window.innerWidth < 1200 ? "tablet" : "laptop";
}

const DEFAULT_METRICS: Record<WidgetKind, MetricId[]> = {
  big_number: ["power_3s", "target_power"],
  number_sparkline: ["hr"],
  zone_bar: ["wbal_pct"],
  log_time_chart: ["power", "hr", "target_power"],
  histogram: ["power"],
  heatmap2d: ["power", "hr"],
  mmp_curve: ["power"],
  workout_strip: ["elapsed"],
  hr_recovery: ["hr"],
  coach_card: [],
};
const DEFAULT_SIZE: Record<WidgetKind, [number, number]> = {
  big_number: [3, 2],
  number_sparkline: [3, 2],
  zone_bar: [6, 1],
  log_time_chart: [8, 3],
  histogram: [4, 3],
  heatmap2d: [4, 3],
  mmp_curve: [4, 3],
  workout_strip: [12, 2],
  hr_recovery: [8, 4],
  coach_card: [4, 3],
};
const DEFAULT_RATE: Record<WidgetKind, number> = {
  big_number: 4,
  number_sparkline: 1,
  zone_bar: 2,
  log_time_chart: 1,
  histogram: 0.5,
  heatmap2d: 0.2,
  mmp_curve: 0.2,
  workout_strip: 1,
  hr_recovery: 2,
  coach_card: 1,
};

export const layoutState = createRoot(() => {
  const initialDevice = defaultDevice();
  const [device, setDevice] = createSignal<Layout["device"]>(initialDevice);
  const initial = backend.layouts.get(initialDevice) ?? backend.layouts.list()[0];
  if (!initial) throw new Error("no layouts");
  const [layout, setLayout] = createStore<Layout>(initial);
  const [pageIdx, setPageIdx] = createSignal(0);
  const [editing, setEditing] = createSignal(false);
  const [selected, setSelected] = createSignal<string | null>(null);
  const [dirty, setDirty] = createSignal(false);
  const [autoSwitch, setAutoSwitch] = createSignal(true);

  const page = (): Page | undefined => layout.pages[pageIdx()];

  // context-aware page switch on phase change (not while editing)
  createEffect(
    on(
      () => ride.status.phase,
      (phase) => {
        if (!autoSwitch() || editing()) return;
        const i = layout.pages.findIndex((p) => p.autoPhase?.includes(phase));
        if (i >= 0) setPageIdx(i);
      },
      { defer: true },
    ),
  );

  const loadDevice = (d: Layout["device"]) => {
    const l = backend.layouts.get(d);
    if (!l) return;
    batch(() => {
      setDevice(d);
      setLayout(reconcile(l));
      setPageIdx(0);
      setDirty(false);
    });
  };

  const mutatePage = (fn: (p: Page) => void) => {
    const i = pageIdx();
    setLayout("pages", i, produce(fn));
    setDirty(true);
  };

  const actions = {
    nextPage(d: 1 | -1) {
      const n = layout.pages.length;
      setPageIdx((i) => (i + d + n) % n);
    },
    goPage(i: number) {
      setPageIdx(Math.max(0, Math.min(layout.pages.length - 1, i)));
    },
    setEditing(v: boolean) {
      setEditing(v);
      if (!v) setSelected(null);
    },
    select: setSelected,
    loadDevice,
    updateWidget(id: string, patch: WidgetPatch) {
      mutatePage((p) => {
        const w = p.widgets.find((x) => x.id === id);
        if (!w) return;
        Object.assign(w, patch);
        // exactOptionalPropertyTypes: "unset" means delete the key, not store undefined
        for (const [k, v] of Object.entries(patch)) if (v === undefined) Reflect.deleteProperty(w, k);
      });
    },
    addWidget(kind: WidgetKind) {
      const [w, h] = DEFAULT_SIZE[kind];
      const id = `${kind}-${Math.random().toString(36).slice(2, 7)}`;
      mutatePage((p) => {
        const bottom = p.widgets.reduce((m, x) => Math.max(m, x.y + x.h), 0);
        p.widgets.push({ id, kind, x: 0, y: bottom, w, h, metrics: [...DEFAULT_METRICS[kind]], rateHz: DEFAULT_RATE[kind] });
      });
      setSelected(id);
    },
    removeWidget(id: string) {
      mutatePage((p) => {
        p.widgets = p.widgets.filter((w) => w.id !== id);
      });
      if (selected() === id) setSelected(null);
    },
    addPage() {
      setLayout(
        "pages",
        produce((ps) => {
          ps.push({ id: `page-${Math.random().toString(36).slice(2, 6)}`, name: `Page ${ps.length + 1}`, widgets: [] });
        }),
      );
      setPageIdx(layout.pages.length - 1);
      setDirty(true);
    },
    removePage() {
      if (layout.pages.length <= 1) return;
      const i = pageIdx();
      setLayout("pages", (ps) => ps.filter((_, j) => j !== i));
      setPageIdx(Math.max(0, i - 1));
      setDirty(true);
    },
    renamePage(name: string) {
      mutatePage((p) => {
        p.name = name;
      });
    },
    togglePhase(phase: NonNullable<Page["autoPhase"]>[number]) {
      mutatePage((p) => {
        const cur = p.autoPhase ?? [];
        p.autoPhase = cur.includes(phase) ? cur.filter((x) => x !== phase) : [...cur, phase];
      });
    },
    save() {
      backend.layouts.save(structuredClone(unwrap(layout)));
      setDirty(false);
    },
    reset() {
      const l = backend.layouts.reset(layout.id);
      setLayout(reconcile(l));
      setPageIdx(0);
      setDirty(false);
    },
    discard() {
      loadDevice(device());
    },
    setAutoSwitch,
  };

  return { layout, device, page, pageIdx, editing, selected, dirty, autoSwitch, actions };
});
