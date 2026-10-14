// Theme preference: dark | light | auto (spec/THEMES.md §Modes). Interim persistence is localStorage behind a
// narrow parser (#88, makes spike defect #4 impossible); it moves to the persistence primitive with #90.
import { useEffect, useSyncExternalStore } from "react";

export type ThemePref = "auto" | "dark" | "light";
export type ThemeMode = "dark" | "light";

const KEY = "crank.theme.v1";
export const THEME_PREFS: readonly ThemePref[] = ["auto", "dark", "light"];

/** Accepts exactly `{"v":1,"pref":"auto"|"dark"|"light"}`; anything else is rejected. */
export function parseTheme(raw: string | null): ThemePref | null {
  if (raw === null) return null;
  try {
    const v: unknown = JSON.parse(raw);
    if (typeof v !== "object" || v === null) return null;
    const o = v as Record<string, unknown>;
    if (o.v !== 1) return null;
    return THEME_PREFS.find((p) => p === o.pref) ?? null;
  } catch {
    return null;
  }
}

function read(): { pref: ThemePref; notice: string | null } {
  let raw: string | null = null;
  try {
    raw = localStorage.getItem(KEY);
  } catch {
    return {
      pref: "auto",
      notice: "Saved appearance could not be read; using Auto.",
    };
  }
  if (raw === null) return { pref: "auto", notice: null };
  const pref = parseTheme(raw);
  return pref
    ? { pref, notice: null }
    : {
        pref: "auto",
        notice: "Saved appearance was invalid and was reset to Auto.",
      };
}

let state = read();
const listeners = new Set<() => void>();
const emit = () => listeners.forEach((l) => l());

export function setThemePref(pref: ThemePref): void {
  let notice: string | null = null;
  try {
    localStorage.setItem(KEY, JSON.stringify({ v: 1, pref }));
  } catch {
    notice = "Appearance could not be saved; it applies until you quit.";
  }
  state = { pref, notice };
  emit();
}

/** Re-read storage (tests, and a future cross-window sync). */
export function reloadTheme(): void {
  state = read();
  emit();
}

const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => {
    listeners.delete(l);
  };
};
export const useThemePref = () => useSyncExternalStore(subscribe, () => state);

const DARK_QUERY = "(prefers-color-scheme: dark)";
/** The OS preference; dark ("pain cave", THEMES.md default) when the platform cannot tell. */
const systemDark = (): boolean =>
  typeof matchMedia === "function" ? matchMedia(DARK_QUERY).matches : true;

/** Keeps `<html data-theme>` on the resolved mode, live (no reload) and following the OS under Auto. */
export function useApplyTheme(): ThemeMode {
  const { pref } = useThemePref();
  const os = useSyncExternalStore((notify) => {
    if (typeof matchMedia !== "function") return () => {};
    const mq = matchMedia(DARK_QUERY);
    mq.addEventListener("change", notify);
    return () => mq.removeEventListener("change", notify);
  }, systemDark);
  const mode: ThemeMode = pref === "auto" ? (os ? "dark" : "light") : pref;
  useEffect(() => {
    document.documentElement.dataset.theme = mode;
  }, [mode]);
  return mode;
}
