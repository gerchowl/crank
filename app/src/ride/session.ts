// The per-ride scope of the window (DESIGN §10 #3, #6; #96): everything that belongs to one ride lives in one
// `RideSession`, made when a ride is loaded (`beginRide`, called with the `ride_load` answer) and disposed when the
// next one begins. `useSession()` returns that one instance, so every component of a ride sees the same history.
import { useSyncExternalStore } from "react";
import type { RideLoaded } from "../bindings";

/** One measured power sample, placed on the workout clock (display only: no derived physiology, #2). */
export interface PowerPoint {
  workoutS: number;
  w: number;
}

/** What one `ride_load` made. */
export class RideSession {
  /** The history. Replaced on every append, never mutated, so a reader (and the render cache) sees the change. */
  power: readonly PowerPoint[] = [];
  disposed = false;
  private lastT = -Infinity;
  private routed = false;

  constructor(readonly loaded: RideLoaded) {}

  get rideId(): string {
    return this.loaded.rideId;
  }
  /** `null` for a free ride. */
  get workoutId(): string | null {
    return this.loaded.workoutId;
  }

  /** Record a sample of session time `t` once: a re-delivered sample (same `t`) is not a second point. */
  record(t: number, workoutS: number, w: number): void {
    if (this.disposed || t <= this.lastT) return;
    this.lastT = t;
    this.power = [...this.power, { workoutS, w }];
    notify();
  }

  /** Run `go` the first time only: a ride that finishes leaves the screen once, however many signals say so. */
  leaveOnce(go: () => void): void {
    if (this.disposed || this.routed) return;
    this.routed = true;
    go();
  }

  dispose(): void {
    this.disposed = true;
    this.power = [];
  }
}

let current: RideSession | null = null;
const listeners = new Set<() => void>();
const notify = () => {
  for (const l of [...listeners]) l();
};

/** A ride was loaded: dispose the previous session and make the one for `loaded`. */
export function beginRide(loaded: RideLoaded): RideSession {
  current?.dispose();
  current = new RideSession(loaded);
  notify();
  return current;
}

/** The ride was unloaded before it started: nothing is left of it, so a late command finds no session. */
export function releaseRide(session: RideSession): void {
  session.dispose();
  if (current === session) current = null;
  notify();
}

const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => {
    listeners.delete(l);
  };
};

/** The current ride's session, `null` until a ride is loaded. */
export const useSession = (): RideSession | null =>
  useSyncExternalStore(subscribe, () => current);

/** The ride's history; re-renders on every appended sample. */
export const usePower = (s: RideSession): readonly PowerPoint[] =>
  useSyncExternalStore(subscribe, () => s.power);
