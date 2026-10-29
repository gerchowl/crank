// The shell's route: four places plus the modes of DESIGN §3. Ride, Onboarding and the Resume sheet are modes:
// no rail, nowhere half-set-up to wander. A tiny external store, not a router library: there is no URL in a
// desktop webview and the route is not persisted (a relaunch goes through the start-screen hand-off).
import { useSyncExternalStore } from "react";

export type Place = "today" | "workouts" | "history" | "settings";
export type Mode = "ride" | "onboarding" | "resume";
export type View = Place | Mode;
/** `sub` selects a section inside a view (Settings: "devices" … DESIGN §5.9). */
export interface Route {
  view: View;
  sub?: string;
}

export const PLACES: readonly Place[] = [
  "today",
  "workouts",
  "history",
  "settings",
];
export const isMode = (v: View): v is Mode =>
  v === "ride" || v === "onboarding" || v === "resume";

let current: Route = { view: "today" };
const listeners = new Set<() => void>();

export function navigate(view: View, sub?: string): void {
  current = sub === undefined ? { view } : { view, sub };
  for (const l of [...listeners]) l();
}
const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => {
    listeners.delete(l);
  };
};
/** The route right now, outside React (an app-scope controller applying a result). */
export const getRoute = (): Route => current;
export const useRoute = (): Route =>
  useSyncExternalStore(subscribe, () => current);
