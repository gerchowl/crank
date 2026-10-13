import { createRenderEffect, createRoot, createSignal } from "solid-js";

export type Mode = "dark" | "light" | "auto";
export type Preset = "graphite" | "ocean" | "ember";

/** Resolved design tokens for canvas / uPlot drawing (they can't read CSS variables). */
export interface ThemeTokens {
  dark: boolean;
  surface0: string;
  surface1: string;
  surface2: string;
  ink: string;
  ink2: string;
  muted: string;
  grid: string;
  axis: string;
  border: string;
  accent: string;
  accentSoft: string;
  target: string;
  power: string;
  hr: string;
  cadence: string;
  dfa: string;
  torque: string;
  zones: string[]; // Z1..Z7 (classic toggle may override)
  seq: string[]; // preset sequential ramp (never classic)
  fatigue: string[]; // 1..5
  divNeg: string;
  divMid: string;
  divPos: string;
  good: string;
  warning: string;
  serious: string;
  critical: string;
  font: string;
}

const STORAGE_KEY = "neo-trainer.theme.v1";

interface Persisted {
  mode: Mode;
  preset: Preset;
  classicZones: boolean;
}

function load(): Persisted {
  const fallback: Persisted = { mode: "dark", preset: "ocean", classicZones: false };
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return fallback;
    const v = JSON.parse(raw) as Partial<Persisted>;
    return {
      mode: v.mode === "light" || v.mode === "auto" || v.mode === "dark" ? v.mode : fallback.mode,
      preset: v.preset === "graphite" || v.preset === "ember" || v.preset === "ocean" ? v.preset : fallback.preset,
      classicZones: v.classicZones === true,
    };
  } catch {
    return fallback;
  }
}

function read(): ThemeTokens {
  const cs = getComputedStyle(document.documentElement);
  const v = (name: string) => cs.getPropertyValue(name).trim();
  return {
    dark: cs.colorScheme.includes("dark"),
    surface0: v("--surface-0"),
    surface1: v("--surface-1"),
    surface2: v("--surface-2"),
    ink: v("--ink-primary"),
    ink2: v("--ink-secondary"),
    muted: v("--ink-muted"),
    grid: v("--grid"),
    axis: v("--axis"),
    border: v("--border"),
    accent: v("--accent"),
    accentSoft: v("--accent-soft"),
    target: v("--target"),
    power: v("--series-power"),
    hr: v("--series-hr"),
    cadence: v("--series-cadence"),
    dfa: v("--series-dfa"),
    torque: v("--series-torque"),
    zones: [1, 2, 3, 4, 5, 6, 7].map((i) => v(`--zone-${i}`)),
    seq: [1, 2, 3, 4, 5, 6, 7].map((i) => v(`--seq-${i}`)),
    fatigue: [1, 2, 3, 4, 5].map((i) => v(`--fatigue-${i}`)),
    divNeg: v("--div-neg"),
    divMid: v("--div-mid"),
    divPos: v("--div-pos"),
    good: v("--status-good"),
    warning: v("--status-warning"),
    serious: v("--status-serious"),
    critical: v("--status-critical"),
    font: v("--font"),
  };
}

/** Global theme state: module-level signals in their own root (app-lifetime singletons). */
export const theme = createRoot(() => {
  const init = load();
  const [mode, setMode] = createSignal<Mode>(init.mode);
  const [preset, setPreset] = createSignal<Preset>(init.preset);
  const [classicZones, setClassicZones] = createSignal(init.classicZones);
  const mq = window.matchMedia("(prefers-color-scheme: dark)");
  const [osDark, setOsDark] = createSignal(mq.matches);
  mq.addEventListener("change", (e) => setOsDark(e.matches));

  const [tokens, setTokens] = createSignal<ThemeTokens>(read(), { equals: false });

  // Apply attributes, then re-read the resolved tokens in the same synchronous pass so every
  // canvas that tracks tokens() redraws with the new colours (live switching, no reload).
  createRenderEffect(() => {
    const root = document.documentElement;
    root.dataset["mode"] = mode();
    root.dataset["theme"] = preset();
    root.dataset["zones"] = classicZones() ? "classic" : "ramp";
    osDark(); // auto mode depends on the OS scheme
    setTokens(read());
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify({ mode: mode(), preset: preset(), classicZones: classicZones() }));
    } catch {
      /* private mode */
    }
  });

  return { mode, setMode, preset, setPreset, classicZones, setClassicZones, tokens };
});

/** Zone colour for a 0-based zone index (clamped). */
export function zoneColor(t: ThemeTokens, idx: number): string {
  return t.zones[Math.max(0, Math.min(t.zones.length - 1, idx))] ?? t.muted;
}

/** Hex token → rgba() with alpha (canvas-safe); non-hex colours are returned unchanged. */
export function withAlpha(color: string, alpha: number): string {
  const m = /^#([0-9a-f]{6})$/i.exec(color);
  if (!m?.[1]) return color;
  const n = parseInt(m[1], 16);
  return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, ${alpha})`;
}
