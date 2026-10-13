// Ride-screen layout state. The layout is a deeply reactive $state proxy, so edit mode can
// mutate widgets in place (x/y/w/h, rateHz, filter…) and only the touched widget reacts:
// a rateHz change re-runs exactly that widget's subscription effect.
import type { Layout, Page, Phase, Widget, WidgetKind, MetricId } from "@neo/shared";
import { backend } from "./backend";

export type DeviceId = "laptop" | "tablet" | "tv";

function pickDevice(): DeviceId {
  const p = new URLSearchParams(location.search).get("layout");
  if (p === "laptop" || p === "tablet" || p === "tv") return p;
  return window.innerWidth < 1200 ? "tablet" : "laptop";
}

function mustGet(id: string): Layout {
  const l = backend.layouts.get(id);
  if (!l) throw new Error(`layout ${id} missing`);
  return l;
}

export const ROWS: Record<DeviceId, number> = { laptop: 8, tablet: 9, tv: 6 };

export const KIND_DEFAULTS: Record<WidgetKind, { metrics: MetricId[]; w: number; h: number; rateHz: number; extra?: Partial<Widget> }> = {
  big_number: { metrics: ["power_3s", "target_power"], w: 3, h: 2, rateHz: 4, extra: { colorByZone: true } },
  number_sparkline: { metrics: ["hr"], w: 3, h: 2, rateHz: 1, extra: { spanS: 300 } },
  zone_bar: { metrics: ["wbal_pct"], w: 6, h: 1, rateHz: 2 },
  log_time_chart: { metrics: ["power", "hr", "target_power"], w: 8, h: 3, rateHz: 1, extra: { spanS: 7200, scale: "log" } },
  histogram: { metrics: ["power"], w: 4, h: 3, rateHz: 0.5, extra: { colorByZone: true } },
  heatmap2d: { metrics: ["power", "hr"], w: 4, h: 3, rateHz: 0.2 },
  mmp_curve: { metrics: ["power"], w: 4, h: 3, rateHz: 0.2 },
  workout_strip: { metrics: ["elapsed"], w: 12, h: 2, rateHz: 1 },
  hr_recovery: { metrics: ["hr"], w: 8, h: 4, rateHz: 2 },
  coach_card: { metrics: [], w: 4, h: 2, rateHz: 1 },
};

const pageParam = new URLSearchParams(location.search).get("page");

class LayoutState {
  device = $state<DeviceId>(pickDevice());
  layout = $state<Layout>(mustGet(this.device));
  pageIndex = $state(Math.max(0, this.layout.pages.findIndex((p) => p.id === pageParam)));
  /** ?page=… pins a page (screenshots); otherwise follow the phase. */
  autoSwitch = $state(pageParam === null);
  editing = $state(false);
  selectedId = $state<string | null>(null);
  /** Snapshot taken when entering edit mode, for Cancel. */
  #before: Layout | null = null;

  rows = $derived(Math.max(ROWS[this.device], ...(this.layout.pages[this.pageIndex]?.widgets ?? []).map((w) => w.y + w.h)));
  page = $derived<Page | undefined>(this.layout.pages[this.pageIndex]);
  selected = $derived<Widget | undefined>(this.page?.widgets.find((w) => w.id === this.selectedId));

  setDevice(d: DeviceId): void {
    this.device = d;
    this.layout = mustGet(d);
    this.pageIndex = Math.min(this.pageIndex, this.layout.pages.length - 1);
  }

  go(delta: number): void {
    const n = this.layout.pages.length;
    this.pageIndex = (((this.pageIndex + delta) % n) + n) % n;
  }

  /** Auto page switch by phase (called from an effect on ride.phase). */
  followPhase(phase: Phase): void {
    if (!this.autoSwitch || this.editing) return;
    const i = this.layout.pages.findIndex((p) => p.autoPhase?.includes(phase));
    if (i >= 0) this.pageIndex = i;
  }

  startEdit(): void {
    this.#before = $state.snapshot(this.layout);
    this.editing = true;
  }

  cancelEdit(): void {
    if (this.#before) this.layout = this.#before;
    this.#before = null;
    this.editing = false;
    this.selectedId = null;
  }

  save(): void {
    backend.layouts.save($state.snapshot(this.layout));
    this.#before = null;
    this.editing = false;
    this.selectedId = null;
  }

  reset(): void {
    this.layout = backend.layouts.reset(this.layout.id);
    this.#before = $state.snapshot(this.layout);
    this.pageIndex = Math.min(this.pageIndex, this.layout.pages.length - 1);
  }

  addWidget(kind: WidgetKind): void {
    const page = this.page;
    if (!page) return;
    const d = KIND_DEFAULTS[kind];
    const y = Math.max(0, ...page.widgets.map((w) => w.y + w.h));
    const id = `${kind}-${Math.random().toString(36).slice(2, 7)}`;
    page.widgets.push({ id, kind, x: 0, y, w: d.w, h: d.h, metrics: [...d.metrics], rateHz: d.rateHz, ...structuredClone(d.extra ?? {}) });
    this.selectedId = id;
  }

  removeWidget(id: string): void {
    const page = this.page;
    if (!page) return;
    const i = page.widgets.findIndex((w) => w.id === id);
    if (i >= 0) page.widgets.splice(i, 1);
    if (this.selectedId === id) this.selectedId = null;
  }

  /** After a drop: push widgets overlapping `moved` (and then each other) downwards. */
  resolveCollisions(moved: Widget): void {
    const page = this.page;
    if (!page) return;
    const overlaps = (a: Widget, b: Widget) => a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
    const queue: Widget[] = [moved];
    for (let guard = 0; queue.length && guard < 200; guard++) {
      const a = queue.shift();
      if (!a) break;
      for (const b of page.widgets) {
        if (b === a || b.id === moved.id || !overlaps(a, b)) continue;
        b.y = a.y + a.h;
        queue.push(b);
      }
    }
  }

  addPage(): void {
    this.layout.pages.push({ id: `page-${Date.now().toString(36)}`, name: `Page ${this.layout.pages.length + 1}`, widgets: [] });
    this.pageIndex = this.layout.pages.length - 1;
  }
}

export const layout = new LayoutState();
