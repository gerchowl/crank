// Theme state lives in a tiny external store; tokens live in CSS (tokens.css, generated and validated).
// Canvas/uPlot can't see CSS variables, so we read them into a JS object (ChartTheme) whenever the
// theme, mode, zone style or the OS colour scheme changes — charts redraw when that object changes.
import { useSyncExternalStore } from "react";
import { createStore } from "../lib/store";

export type ThemeName = "graphite" | "ocean" | "ember";
export type ThemeMode = "dark" | "light" | "auto";
export interface ThemePrefs { theme: ThemeName; mode: ThemeMode; classicZones: boolean }

export interface ChartTheme {
  dark: boolean;
  surface0: string; surface1: string; surface2: string;
  ink: string; ink2: string; muted: string; border: string; grid: string;
  accent: string; accentSoft: string;
  series: { power: string; hr: string; cadence: string; dfa: string; torque: string };
  target: string;
  zones: string[]; seq: string[]; fatigue: string[];
  divNeg: string; divMid: string; divPos: string;
  status: { good: string; warning: string; serious: string; critical: string };
  font: string;
}

const KEY = "neo-trainer.theme.v1";
const THEMES: readonly ThemeName[] = ["graphite", "ocean", "ember"];
const MODES: readonly ThemeMode[] = ["dark", "light", "auto"];

function loadPrefs(): ThemePrefs {
  const q = new URLSearchParams(location.search);
  let saved: Partial<ThemePrefs> = {};
  try {
    saved = JSON.parse(localStorage.getItem(KEY) ?? "{}") as Partial<ThemePrefs>;
  } catch {
    /* ignore */
  }
  const t = q.get("theme") ?? saved.theme;
  const m = q.get("mode") ?? saved.mode;
  return {
    theme: THEMES.find((x) => x === t) ?? "ocean",
    mode: MODES.find((x) => x === m) ?? "dark",
    classicZones: q.get("zones") === "classic" || (saved.classicZones ?? false),
  };
}

export const themePrefs = createStore<ThemePrefs>(loadPrefs());

function apply(p: ThemePrefs) {
  const root = document.documentElement;
  root.dataset["theme"] = p.theme;
  root.dataset["mode"] = p.mode;
  root.dataset["zones"] = p.classicZones ? "classic" : "sequential";
}

function readTheme(): ChartTheme {
  const cs = getComputedStyle(document.documentElement);
  const v = (n: string) => cs.getPropertyValue(n).trim();
  const list = (prefix: string, n: number) => Array.from({ length: n }, (_, i) => v(`${prefix}${i + 1}`));
  return {
    dark: cs.colorScheme.includes("dark"),
    surface0: v("--surface-0"), surface1: v("--surface-1"), surface2: v("--surface-2"),
    ink: v("--ink-primary"), ink2: v("--ink-secondary"), muted: v("--ink-muted"), border: v("--border"), grid: v("--grid"),
    accent: v("--accent"), accentSoft: v("--accent-soft"),
    series: { power: v("--series-power"), hr: v("--series-hr"), cadence: v("--series-cadence"), dfa: v("--series-dfa"), torque: v("--series-torque") },
    target: v("--target"),
    zones: list("--zone-", 7), seq: list("--seq-", 7), fatigue: list("--fatigue-", 5),
    divNeg: v("--div-neg"), divMid: v("--div-mid"), divPos: v("--div-pos"),
    status: { good: v("--status-good"), warning: v("--status-warning"), serious: v("--status-serious"), critical: v("--status-critical") },
    font: getComputedStyle(document.body).fontFamily,
  };
}

apply(themePrefs.get());
const chartTheme = createStore<ChartTheme>(readTheme());
const refresh = () => {
  apply(themePrefs.get());
  chartTheme.set(readTheme());
};
themePrefs.subscribe(() => {
  try {
    localStorage.setItem(KEY, JSON.stringify(themePrefs.get()));
  } catch {
    /* ignore */
  }
  refresh();
});
matchMedia("(prefers-color-scheme: dark)").addEventListener("change", refresh);

export function useThemePrefs(): ThemePrefs {
  return useSyncExternalStore(themePrefs.subscribe, themePrefs.get);
}

/** JS view of the current tokens; a new object on every theme change (use it as an effect dependency to redraw). */
export function useChartTheme(): ChartTheme {
  return useSyncExternalStore(chartTheme.subscribe, chartTheme.get);
}

/** Map a 0-based zone index to its ramp colour (7-step ramp; HR's 5 zones use steps 1,2,4,5,6... spread evenly). */
export function zoneColor(t: ChartTheme, zoneIdx: number, zoneCount = 7): string {
  const i = zoneCount === 7 ? zoneIdx : Math.round((zoneIdx * 6) / Math.max(1, zoneCount - 1));
  return t.zones[Math.max(0, Math.min(6, i))] ?? t.muted;
}
