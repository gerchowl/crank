// Theme = data (mode × preset × classic-zones). The DOM attributes drive the CSS tokens;
// canvases and uPlot can't see CSS variables, so `theme.colors` is a JS snapshot of the
// resolved tokens. It is $state.raw: every chart effect that reads it redraws on change.

export type Mode = "dark" | "light" | "auto";
export type Preset = "ocean" | "graphite" | "ember";

export interface ChartColors {
  dark: boolean;
  surface: string;
  surface2: string;
  ink: string;
  inkSecondary: string;
  muted: string;
  grid: string;
  axis: string;
  accent: string;
  power: string;
  hr: string;
  cadence: string;
  dfa: string;
  torque: string;
  target: string;
  zones: string[]; // Z1..Z7
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

interface Stored {
  mode: Mode;
  preset: Preset;
  classicZones: boolean;
  oled: boolean;
}

const KEY = "neo-trainer.theme.v1";

function load(): Stored {
  const fallback: Stored = { mode: "dark", preset: "ocean", classicZones: false, oled: false };
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return fallback;
    const v = JSON.parse(raw) as Partial<Stored>;
    return { ...fallback, ...v };
  } catch {
    return fallback;
  }
}

function read(): ChartColors {
  const cs = getComputedStyle(document.documentElement);
  const v = (name: string) => cs.getPropertyValue(name).trim();
  return {
    dark: cs.colorScheme.includes("dark"),
    surface: v("--surface-1"),
    surface2: v("--surface-2"),
    ink: v("--ink-primary"),
    inkSecondary: v("--ink-secondary"),
    muted: v("--ink-muted"),
    grid: v("--grid"),
    axis: v("--axis"),
    accent: v("--accent"),
    power: v("--series-power"),
    hr: v("--series-hr"),
    cadence: v("--series-cadence"),
    dfa: v("--series-dfa"),
    torque: v("--series-torque"),
    target: v("--target"),
    zones: [1, 2, 3, 4, 5, 6, 7].map((i) => v(`--zone-${i}`)),
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

class Theme {
  mode = $state<Mode>("dark");
  preset = $state<Preset>("ocean");
  classicZones = $state(false);
  oled = $state(false);
  /** Resolved token values for canvas/uPlot. Replaced wholesale on every theme change. */
  colors = $state.raw<ChartColors>(read());
  /** OS preference, tracked so Auto reacts live. */
  osDark = $state(matchMedia("(prefers-color-scheme: dark)").matches);

  constructor() {
    const s = load();
    this.mode = s.mode;
    this.preset = s.preset;
    this.classicZones = s.classicZones;
    this.oled = s.oled;
    matchMedia("(prefers-color-scheme: dark)").addEventListener("change", (e) => {
      this.osDark = e.matches;
    });
  }

  /** Call from a root-level $effect: syncs attributes, persists, re-reads tokens. */
  apply(): void {
    const el = document.documentElement;
    el.dataset["mode"] = this.mode;
    el.dataset["theme"] = this.preset;
    el.dataset["zones"] = this.classicZones ? "classic" : "ramp";
    el.dataset["oled"] = String(this.oled);
    el.dataset["osDark"] = String(this.osDark); // dependency: Auto mode flips with the OS
    const stored: Stored = { mode: this.mode, preset: this.preset, classicZones: this.classicZones, oled: this.oled };
    localStorage.setItem(KEY, JSON.stringify(stored));
    this.colors = read();
  }
}

export const theme = new Theme();

/** Fixed series identity by metric (VIEWS §0): power→1, HR→2, cadence→3, DFA→4, torque→5. */
export function seriesColor(c: ChartColors, metric: string): string {
  if (metric === "target_power" || metric === "target_cadence") return c.target;
  if (metric.startsWith("power") || metric === "np" || metric === "w_per_kg" || metric === "pct_ftp" || metric === "wbal_pct" || metric === "kj") return c.power;
  if (metric.startsWith("hr") || metric === "rr" || metric === "rmssd") return c.hr;
  if (metric === "cadence") return c.cadence;
  if (metric === "dfa_a1") return c.dfa;
  return c.inkSecondary;
}

/** Map a 0-based zone index of an n-zone model onto the 7-step ramp. */
export function zoneColor(c: ChartColors, index: number, n = 7): string {
  const step = n <= 1 ? 0 : Math.round((index * 6) / (n - 1));
  return c.zones[Math.max(0, Math.min(6, step))] ?? c.muted;
}

/** Linear interpolation across a list of hex colours, t ∈ [0, 1]. */
export function rampAt(stops: readonly string[], t: number): string {
  if (stops.length === 0) return "#888888";
  const x = Math.max(0, Math.min(1, t)) * (stops.length - 1);
  const i = Math.floor(x);
  const a = hexRgb(stops[i] ?? stops[0] ?? "#888888");
  const b = hexRgb(stops[Math.min(stops.length - 1, i + 1)] ?? stops[0] ?? "#888888");
  const f = x - i;
  const mix = (p: number, q: number) => Math.round(p + (q - p) * f);
  return `rgb(${mix(a[0], b[0])},${mix(a[1], b[1])},${mix(a[2], b[2])})`;
}

export function hexRgb(hex: string): [number, number, number] {
  const h = hex.replace("#", "");
  const n = parseInt(h.length === 3 ? h.replace(/(.)/g, "$1$1") : h.slice(0, 6), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

export function withAlpha(hex: string, a: number): string {
  const [r, g, b] = hexRgb(hex);
  return `rgba(${r},${g},${b},${a})`;
}
