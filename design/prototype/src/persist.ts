// Everything the prototype persists goes through a validator (DESIGN §10 #4): invalid or old
// data falls back to the default and reports it; nothing unvalidated reaches the UI.
import type { MetricId } from "@crank/shared";
import { createStore } from "./store";

export type Theme = "dark" | "light";
export type Density = "focus" | "standard" | "detail";
export const SLOT_METRICS = ["hr", "cadence", "wbal_pct", "dfa_a1", "hr_vs_typical", "power_10s", "kj"] as const satisfies readonly MetricId[];
export type SlotMetric = (typeof SLOT_METRICS)[number];
export interface RideScreenConfig { density: Density; slotA: SlotMetric; slotB: SlotMetric }
export const DEFAULT_RIDE_SCREEN: RideScreenConfig = { density: "standard", slotA: "hr", slotB: "cadence" };

const isSlot = (v: unknown): v is SlotMetric => SLOT_METRICS.includes(v as SlotMetric);
function parseRideScreen(v: unknown): RideScreenConfig | null {
  if (typeof v !== "object" || v === null) return null;
  const o = v as Record<string, unknown>;
  if (!(o.density === "focus" || o.density === "standard" || o.density === "detail")) return null;
  if (!isSlot(o.slotA) || !isSlot(o.slotB) || o.slotA === o.slotB) return null;
  return { density: o.density, slotA: o.slotA, slotB: o.slotB };
}
const parseTheme = (v: unknown): Theme | null => (v === "dark" || v === "light" ? v : null);

/** Notices about data that was dropped while loading (shown in Settings). */
export const persistNotices = createStore<string[]>([]);

function load<T>(key: string, parse: (v: unknown) => T | null, fallback: T): T {
  const raw = (() => {
    try {
      return localStorage.getItem(key);
    } catch {
      return null;
    }
  })();
  if (raw === null) return fallback;
  let parsed: T | null = null;
  try {
    parsed = parse(JSON.parse(raw));
  } catch {
    parsed = null;
  }
  if (parsed === null) persistNotices.set((n) => [...n, `Saved "${key}" was invalid and was reset to the default.`]);
  return parsed ?? fallback;
}
function save(key: string, v: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(v));
  } catch {
    persistNotices.set((n) => [...n, `Could not save "${key}".`]);
  }
}

const RIDE_KEY = "crank.design.rideScreen.v1";
const THEME_KEY = "crank.design.theme.v1";
export const rideScreen = createStore<RideScreenConfig>(load(RIDE_KEY, parseRideScreen, DEFAULT_RIDE_SCREEN));
export const theme = createStore<Theme>(load(THEME_KEY, parseTheme, "dark"));

export function saveRideScreen(c: RideScreenConfig): string | null {
  if (!parseRideScreen(c)) return "The two slots must show different metrics.";
  rideScreen.set(c);
  save(RIDE_KEY, c);
  return null;
}
export function setTheme(t: Theme) {
  theme.set(t);
  save(THEME_KEY, t);
}
