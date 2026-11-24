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
/**
 * Asked before the rider leaves the place they are in through the rail (`leave`): the workout editor's unsaved
 * changes (DESIGN §5.7). It answers `true` when it takes over: it asks, and calls `go` once the rider chose to leave.
 * App-scope routing (a ride load completing, a mode) never asks: it calls `navigate`.
 */
export type LeaveGuard = (go: () => void) => boolean;
let guard: LeaveGuard | null = null;
/** Install the guard (one at a time); returns its removal. */
export function setLeaveGuard(g: LeaveGuard): () => void {
  guard = g;
  return () => {
    if (guard === g) guard = null;
  };
}
/** The rider asks to go elsewhere (the rail): the guard may ask first. */
export function leave(view: View, sub?: string): void {
  const go = () => navigate(view, sub);
  if (!guard?.(go)) go();
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
