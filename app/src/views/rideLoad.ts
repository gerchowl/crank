// The load of a ride, owned at app scope (#320). "A ride load is in flight" is the core's truth, not a view's: the
// view that asked can unmount (the rider goes to Settings) while `ride_load` is still connecting devices, and its
// answer must still be applied exactly once, to the workout that was asked for. So the load lives here, next to the
// ride session and the router, and views only read it.
//
// When it completes the result is applied once:
// - success: the ride session begins (`beginRide`, core truth: the core now holds this ride), and the rider is routed
//   into it only if they are still on the Workouts view that asked. Anywhere else they are not pulled away: the
//   state stays `loaded` and a non-modal notice offers "Go to ride" (`RideLoadNotice`).
// - refusal: kept as `refused` for the workout it was for; the Workouts view shows it under that workout, and the
//   notice shows it elsewhere.
import { useSyncExternalStore } from "react";
import { api } from "../api";
import { commands } from "../bindings";
import type { ApiError, RideLoaded } from "../bindings";
import { beginRide } from "../ride/session";
import { getRoute, navigate } from "../router";

export type RideLoadState =
  | { phase: "idle" }
  | { phase: "loading"; workoutId: string; name: string }
  | { phase: "loaded"; workoutId: string; name: string; loaded: RideLoaded }
  | { phase: "refused"; workoutId: string; name: string; error: ApiError };

let state: RideLoadState = { phase: "idle" };
const listeners = new Set<() => void>();
const set = (next: RideLoadState) => {
  state = next;
  for (const l of [...listeners]) l();
};
const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => {
    listeners.delete(l);
  };
};

/** The load in flight, or its unseen result. */
export const useRideLoad = (): RideLoadState =>
  useSyncExternalStore(subscribe, () => state);

/**
 * Ask the core to load `workoutId`. One at a time: while a load is in flight another is not started (the core would
 * refuse it as well: "another ride is loading or loaded").
 */
export async function startRideLoad(
  workoutId: string,
  name: string,
): Promise<void> {
  if (state.phase === "loading") return;
  set({ phase: "loading", workoutId, name });
  const outcome = await api(commands.rideLoad(workoutId));
  if (outcome.status === "error") {
    set({ phase: "refused", workoutId, name, error: outcome.error });
    return;
  }
  beginRide(outcome.data);
  if (getRoute().view === "workouts") {
    set({ phase: "idle" });
    navigate("ride");
  } else {
    set({ phase: "loaded", workoutId, name, loaded: outcome.data });
  }
}

/** Forget the result (a pick of another workout, the notice's dismiss). */
export function dismissRideLoad(): void {
  if (state.phase !== "loading") set({ phase: "idle" });
}

/** The notice's "Go to ride". */
export function goToRide(): void {
  if (state.phase === "loaded") set({ phase: "idle" });
  navigate("ride");
}

/** Test-only: back to a clean state between tests. */
export function resetRideLoad(): void {
  set({ phase: "idle" });
}
