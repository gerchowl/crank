// Pure layout edits (layouts are data). Every edit returns a new Layout; untouched widgets keep
// their identity, so their subscriptions stay put.
import type { Layout, MetricId, Page, Widget, WidgetKind } from "@neo/shared";

export const COLS = 12;
export const rowsOf = (l: Layout): number => (l.device === "tablet" ? 9 : l.device === "tv" ? 6 : 8);

export type LayoutAction =
  | { type: "reset"; layout: Layout }
  | { type: "move"; page: number; id: string; x: number; y: number }
  | { type: "resize"; page: number; id: string; w: number; h: number }
  | { type: "update"; page: number; id: string; patch: Partial<Widget> }
  | { type: "add"; page: number; widget: Widget }
  | { type: "remove"; page: number; id: string }
  | { type: "add_page"; name: string }
  | { type: "rename_page"; page: number; name: string }
  | { type: "remove_page"; page: number };

const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));

function mapPage(l: Layout, i: number, f: (p: Page) => Page): Layout {
  return { ...l, pages: l.pages.map((p, k) => (k === i ? f(p) : p)) };
}
function mapWidget(l: Layout, page: number, id: string, f: (w: Widget) => Widget): Layout {
  return mapPage(l, page, (p) => ({ ...p, widgets: p.widgets.map((w) => (w.id === id ? f(w) : w)) }));
}

export function layoutReducer(l: Layout, a: LayoutAction): Layout {
  const rows = rowsOf(l);
  switch (a.type) {
    case "reset":
      return a.layout;
    case "move":
      return mapWidget(l, a.page, a.id, (w) => ({ ...w, x: clamp(a.x, 0, COLS - w.w), y: clamp(a.y, 0, rows - w.h) }));
    case "resize":
      return mapWidget(l, a.page, a.id, (w) => ({ ...w, w: clamp(a.w, 1, COLS - w.x), h: clamp(a.h, 1, rows - w.y) }));
    case "update":
      return mapWidget(l, a.page, a.id, (w) => ({ ...w, ...a.patch }));
    case "add":
      return mapPage(l, a.page, (p) => ({ ...p, widgets: [...p.widgets, a.widget] }));
    case "remove":
      return mapPage(l, a.page, (p) => ({ ...p, widgets: p.widgets.filter((w) => w.id !== a.id) }));
    case "add_page":
      return { ...l, pages: [...l.pages, { id: `page-${String(Date.now())}`, name: a.name, widgets: [] }] };
    case "rename_page":
      return mapPage(l, a.page, (p) => ({ ...p, name: a.name }));
    case "remove_page":
      return l.pages.length > 1 ? { ...l, pages: l.pages.filter((_, k) => k !== a.page) } : l;
  }
}

/** First free w×h slot on a page (row-major), or the bottom-left if full. */
export function freeSlot(p: Page, w: number, h: number, rows: number): { x: number; y: number } {
  const used = (x: number, y: number) => p.widgets.some((o) => x < o.x + o.w && x + w > o.x && y < o.y + o.h && y + h > o.y);
  for (let y = 0; y + h <= rows; y++) for (let x = 0; x + w <= COLS; x++) if (!used(x, y)) return { x, y };
  return { x: 0, y: Math.max(0, rows - h) };
}

export const DEFAULT_SIZE: Record<WidgetKind, [number, number]> = {
  big_number: [3, 2], number_sparkline: [3, 2], zone_bar: [6, 1], log_time_chart: [8, 3], histogram: [4, 3],
  heatmap2d: [4, 3], mmp_curve: [4, 3], workout_strip: [12, 2], hr_recovery: [8, 4], coach_card: [4, 3],
};

export function defaultMetrics(kind: WidgetKind, metric: MetricId): MetricId[] {
  switch (kind) {
    case "log_time_chart":
      return metric.startsWith("power") ? [metric, "target_power"] : [metric];
    case "heatmap2d":
      return ["power", metric === "power" ? "hr" : metric];
    case "mmp_curve":
      return ["power"];
    case "workout_strip":
      return ["elapsed"];
    case "hr_recovery":
      return ["hr"];
    case "coach_card":
      return [];
    default:
      return [metric];
  }
}
