import type { Layout, Page, Widget } from "./contract";

/*
 * Built-in layouts. Grid: 12 columns; rows are abstract units (laptop 8 rows, tablet 9, tv 6).
 * Widget metric conventions:
 *   big_number / number_sparkline : [value, target?]   (target → Δ display)
 *   zone_bar                       : [value, target?]   (value vs target band / zones)
 *   log_time_chart                 : [series...]        (target_power drawn as overlay)
 *   histogram                      : [metric]
 *   heatmap2d                      : [x, y]
 *   mmp_curve                      : ["power"]
 *   workout_strip                  : ["elapsed"]        (uses ride.current() for the profile)
 *   hr_recovery                    : ["hr"]             (polls hrRecovery())
 *   coach_card                     : []                 (onNudge / coach.decisions())
 */

const W = (id: string, kind: Widget["kind"], x: number, y: number, w: number, h: number, metrics: Widget["metrics"], rateHz: number, extra: Partial<Widget> = {}): Widget => ({
  id,
  kind,
  x,
  y,
  w,
  h,
  metrics,
  rateHz,
  ...extra,
});

function laptop(): Layout {
  const interval: Page = {
    id: "interval",
    name: "Interval",
    autoPhase: ["warmup", "work", "cooldown", "free"],
    widgets: [
      W("power", "big_number", 0, 0, 4, 2, ["power_3s", "target_power"], 4, { colorByZone: true }),
      W("countdown", "big_number", 4, 0, 3, 2, ["block_remaining"], 4),
      W("hr", "number_sparkline", 7, 0, 3, 2, ["hr"], 1, { colorByZone: true, spanS: 300, filter: [{ kind: "hold" }] }),
      W("cadence", "big_number", 10, 0, 2, 2, ["cadence", "target_cadence"], 2, { filter: [{ kind: "ema", tauS: 2 }] }),
      W("wbal", "zone_bar", 0, 2, 12, 1, ["wbal_pct"], 2, { colorByZone: true }),
      W("chart", "log_time_chart", 0, 3, 8, 3, ["power", "hr", "target_power"], 1, { spanS: 7200, scale: "log" }),
      W("coach", "coach_card", 8, 3, 4, 3, [], 1),
      W("strip", "workout_strip", 0, 6, 12, 2, ["elapsed"], 1),
    ],
  };
  const recovery: Page = {
    id: "recovery",
    name: "Recovery",
    autoPhase: ["recovery"],
    widgets: [
      W("hrrec", "hr_recovery", 0, 0, 8, 6, ["hr"], 2),
      W("rec-hr", "big_number", 8, 0, 4, 2, ["hr"], 1, { colorByZone: true, filter: [{ kind: "hold" }] }),
      W("rec-power", "big_number", 8, 2, 4, 2, ["power_3s", "target_power"], 2, { colorByZone: true }),
      W("rec-left", "big_number", 8, 4, 4, 2, ["block_remaining"], 1),
      W("rec-next", "workout_strip", 0, 6, 12, 2, ["elapsed"], 1, { spanS: 900 }),
    ],
  };
  const nerd: Page = {
    id: "nerd",
    name: "Nerd",
    widgets: [
      W("hist-power", "histogram", 0, 0, 4, 3, ["power"], 0.5, { colorByZone: true }),
      W("hist-hr", "histogram", 4, 0, 4, 3, ["hr"], 0.5, { colorByZone: true }),
      W("mmp", "mmp_curve", 8, 0, 4, 3, ["power"], 0.2),
      W("heat-hr", "heatmap2d", 0, 3, 4, 3, ["power", "hr"], 0.2),
      W("heat-cad", "heatmap2d", 4, 3, 4, 3, ["power", "cadence"], 0.2),
      W("dfa", "number_sparkline", 8, 3, 2, 2, ["dfa_a1"], 1, { spanS: 600 }),
      W("rmssd", "number_sparkline", 10, 3, 2, 2, ["rmssd"], 1, { spanS: 600 }),
      W("decoupling", "big_number", 8, 5, 2, 1, ["decoupling"], 0.5),
      W("hrvt", "big_number", 10, 5, 2, 1, ["hr_vs_typical"], 0.5, { filter: [{ kind: "ema", tauS: 10 }] }),
      W("np", "big_number", 0, 6, 3, 2, ["np"], 1),
      W("tss", "big_number", 3, 6, 3, 2, ["tss"], 1),
      W("if", "big_number", 6, 6, 3, 2, ["if"], 1),
      W("compliance", "zone_bar", 9, 6, 3, 2, ["compliance"], 0.5),
    ],
  };
  return { id: "laptop", name: "Laptop", device: "laptop", pages: [interval, recovery, nerd] };
}

function tablet(): Layout {
  return {
    id: "tablet",
    name: "Tablet",
    device: "tablet",
    pages: [
      {
        id: "interval",
        name: "Interval",
        autoPhase: ["warmup", "work", "cooldown", "free"],
        widgets: [
          W("power", "big_number", 0, 0, 6, 3, ["power_3s", "target_power"], 4, { colorByZone: true }),
          W("countdown", "big_number", 6, 0, 6, 3, ["block_remaining"], 4),
          W("hr", "number_sparkline", 0, 3, 4, 2, ["hr"], 1, { colorByZone: true, spanS: 300, filter: [{ kind: "hold" }] }),
          W("cadence", "big_number", 4, 3, 4, 2, ["cadence", "target_cadence"], 2),
          W("wbal", "zone_bar", 8, 3, 4, 2, ["wbal_pct"], 2, { colorByZone: true }),
          W("strip", "workout_strip", 0, 5, 12, 2, ["elapsed"], 1),
          W("coach", "coach_card", 0, 7, 12, 2, [], 1),
        ],
      },
      {
        id: "recovery",
        name: "Recovery",
        autoPhase: ["recovery"],
        widgets: [
          W("hrrec", "hr_recovery", 0, 0, 12, 6, ["hr"], 2),
          W("rec-hr", "big_number", 0, 6, 6, 3, ["hr"], 1, { colorByZone: true, filter: [{ kind: "hold" }] }),
          W("rec-power", "big_number", 6, 6, 6, 3, ["power_3s", "target_power"], 2, { colorByZone: true }),
        ],
      },
      {
        id: "nerd",
        name: "Nerd",
        widgets: [
          W("chart", "log_time_chart", 0, 0, 12, 3, ["power", "hr", "target_power"], 1, { spanS: 7200, scale: "fisheye" }),
          W("hist-power", "histogram", 0, 3, 6, 3, ["power"], 0.5, { colorByZone: true }),
          W("mmp", "mmp_curve", 6, 3, 6, 3, ["power"], 0.2),
          W("heat-hr", "heatmap2d", 0, 6, 6, 3, ["power", "hr"], 0.2),
          W("dfa", "number_sparkline", 6, 6, 6, 3, ["dfa_a1"], 1, { spanS: 600 }),
        ],
      },
    ],
  };
}

function tv(): Layout {
  return {
    id: "tv",
    name: "TV",
    device: "tv",
    pages: [
      {
        id: "interval",
        name: "Interval",
        autoPhase: ["warmup", "work", "cooldown", "free"],
        widgets: [
          W("power", "big_number", 0, 0, 5, 3, ["power_3s", "target_power"], 4, { colorByZone: true }),
          W("countdown", "big_number", 5, 0, 4, 3, ["block_remaining"], 4),
          W("hr", "big_number", 9, 0, 3, 3, ["hr"], 1, { colorByZone: true, filter: [{ kind: "hold" }] }),
          W("strip", "workout_strip", 0, 3, 9, 2, ["elapsed"], 1),
          W("coach", "coach_card", 9, 3, 3, 3, [], 1),
          W("wbal", "zone_bar", 0, 5, 9, 1, ["wbal_pct"], 2, { colorByZone: true }),
        ],
      },
      {
        id: "recovery",
        name: "Recovery",
        autoPhase: ["recovery"],
        widgets: [
          W("hrrec", "hr_recovery", 0, 0, 9, 6, ["hr"], 2),
          W("rec-hr", "big_number", 9, 0, 3, 3, ["hr"], 1, { colorByZone: true, filter: [{ kind: "hold" }] }),
          W("rec-left", "big_number", 9, 3, 3, 3, ["block_remaining"], 1),
        ],
      },
      {
        id: "nerd",
        name: "Nerd",
        widgets: [
          W("chart", "log_time_chart", 0, 0, 8, 3, ["power", "hr", "target_power"], 1, { spanS: 7200, scale: "log" }),
          W("mmp", "mmp_curve", 8, 0, 4, 3, ["power"], 0.2),
          W("hist-hr", "histogram", 0, 3, 4, 3, ["hr"], 0.5, { colorByZone: true }),
          W("heat-cad", "heatmap2d", 4, 3, 4, 3, ["power", "cadence"], 0.2),
          W("sparkline-power", "number_sparkline", 8, 3, 4, 3, ["power_30s"], 1, { spanS: 600 }),
        ],
      },
    ],
  };
}

export function defaultLayouts(): Layout[] {
  return [laptop(), tablet(), tv()];
}

/** Minimal Storage subset (localStorage-compatible). */
export interface StorageLike {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

export function memoryStorage(): StorageLike {
  const m = new Map<string, string>();
  return {
    getItem: (k) => m.get(k) ?? null,
    setItem: (k, v) => void m.set(k, v),
    removeItem: (k) => void m.delete(k),
  };
}

export function browserStorage(): StorageLike | null {
  try {
    const ls = (globalThis as { localStorage?: StorageLike }).localStorage;
    if (!ls) return null;
    const k = "__neo_probe__";
    ls.setItem(k, "1");
    ls.removeItem(k);
    return ls;
  } catch {
    return null;
  }
}

export const LAYOUT_STORAGE_KEY = "neo-trainer.layouts.v1";

/** Saved layouts overlay the built-ins by id; custom ids are appended. */
export class LayoutStore {
  constructor(private storage: StorageLike) {}

  private saved(): Layout[] {
    try {
      const raw = this.storage.getItem(LAYOUT_STORAGE_KEY);
      const v: unknown = raw ? JSON.parse(raw) : [];
      return Array.isArray(v) ? (v as Layout[]) : [];
    } catch {
      return [];
    }
  }

  private write(ls: Layout[]): void {
    try {
      this.storage.setItem(LAYOUT_STORAGE_KEY, JSON.stringify(ls));
    } catch {
      /* quota / private mode: ignore */
    }
  }

  list(): Layout[] {
    const saved = this.saved();
    const out = defaultLayouts().map((d) => saved.find((s) => s.id === d.id) ?? d);
    for (const s of saved) if (!out.some((o) => o.id === s.id)) out.push(s);
    return structuredClone(out);
  }

  get(id: string): Layout | undefined {
    return this.list().find((l) => l.id === id);
  }

  save(l: Layout): void {
    const saved = this.saved().filter((s) => s.id !== l.id);
    saved.push(structuredClone(l));
    this.write(saved);
  }

  reset(id: string): Layout {
    const def = defaultLayouts().find((d) => d.id === id);
    this.write(this.saved().filter((s) => s.id !== id));
    if (!def) throw new Error(`No built-in layout '${id}' (custom layout removed)`);
    return def;
  }
}
