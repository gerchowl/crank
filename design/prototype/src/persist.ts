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

// ---------- coach autonomy (DESIGN §7.1) ----------
export type Autonomy = "ask" | "auto_decreases" | "off";
export const autonomy = createStore<Autonomy>(load("crank.design.autonomy.v1", (v) => (v === "ask" || v === "auto_decreases" || v === "off" ? v : null), "ask"));
export function setAutonomy(a: Autonomy) {
  autonomy.set(a);
  save("crank.design.autonomy.v1", a);
}

// ---------- controls: five abstract ride actions (DESIGN §7.2) ----------
export const ACTIONS = ["coach", "keep", "pause", "skip", "minus", "plus"] as const;
export type Action = (typeof ACTIONS)[number];
export type Bindings = Record<Action, string[]>;
export const DEFAULT_BINDINGS: Bindings = {
  coach: ["Enter", "y", "u"],
  keep: ["n", "Backspace"],
  pause: [" "],
  skip: ["s"],
  minus: ["-"],
  plus: ["+", "="],
};
function parseBindings(v: unknown): Bindings | null {
  if (typeof v !== "object" || v === null) return null;
  const o = v as Record<string, unknown>;
  const out = {} as Bindings;
  const seen = new Set<string>();
  for (const a of ACTIONS) {
    const keys = o[a];
    if (!Array.isArray(keys) || !keys.every((k): k is string => typeof k === "string" && k.length > 0 && k.length <= 24)) return null;
    for (const k of keys) {
      if (seen.has(k)) return null; // one key, one action
      seen.add(k);
    }
    out[a] = keys;
  }
  return out;
}
export const bindings = createStore<Bindings>(load("crank.design.bindings.v1", parseBindings, DEFAULT_BINDINGS));
export function saveBindings(b: Bindings): string | null {
  if (!parseBindings(b)) return "Each key can trigger only one action.";
  bindings.set(b);
  save("crank.design.bindings.v1", b);
  return null;
}
export const actionFor = (b: Bindings, key: string): Action | null => ACTIONS.find((a) => b[a].includes(key) || b[a].includes(key.toLowerCase())) ?? null;

// ---------- sound (DESIGN §7.3) ----------
export const SOUND_EVENTS = ["block", "countdown", "coach", "applied", "warning", "reconnected"] as const;
export type SoundEvent = (typeof SOUND_EVENTS)[number];
export interface SoundConfig { volume: number; muted: SoundEvent[] }
export const DEFAULT_SOUND: SoundConfig = { volume: 0.6, muted: [] };
function parseSound(v: unknown): SoundConfig | null {
  if (typeof v !== "object" || v === null) return null;
  const o = v as Record<string, unknown>;
  if (typeof o.volume !== "number" || !(o.volume >= 0 && o.volume <= 1)) return null;
  if (!Array.isArray(o.muted) || !o.muted.every((m) => SOUND_EVENTS.includes(m as SoundEvent))) return null;
  return { volume: o.volume, muted: o.muted as SoundEvent[] };
}
export const sound = createStore<SoundConfig>(load("crank.design.sound.v1", parseSound, DEFAULT_SOUND));
export function saveSound(c: SoundConfig) {
  if (!parseSound(c)) return;
  sound.set(c);
  save("crank.design.sound.v1", c);
}

// ---------- screen distance (DESIGN §6.2) ----------
export const distanceM = createStore<number>(load("crank.design.distance.v1", (v) => (typeof v === "number" && v >= 0.4 && v <= 3 ? v : null), 1.0));
export function setDistance(m: number) {
  if (!(m >= 0.4 && m <= 3)) return;
  distanceM.set(m);
  save("crank.design.distance.v1", m);
}

// ---------- onboarding progress (DESIGN §5.8) ----------
export const ONBOARDING_STEPS = ["about", "background", "numbers", "health", "devices", "baseline", "result"] as const;
export type OnboardingStep = (typeof ONBOARDING_STEPS)[number];
export const onboardingStep = createStore<OnboardingStep | null>(
  load("crank.design.onboarding.v1", (v) => (ONBOARDING_STEPS.includes(v as OnboardingStep) ? (v as OnboardingStep) : null), null),
);
export function saveOnboardingStep(s: OnboardingStep | null) {
  onboardingStep.set(s);
  if (s !== null) save("crank.design.onboarding.v1", s);
  else
    try {
      localStorage.removeItem("crank.design.onboarding.v1");
    } catch {
      /* storage unavailable: progress simply isn't kept */
    }
}
