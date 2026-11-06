// The per-ride scope of the window (DESIGN §10 #3, #6; #96): everything that belongs to one ride lives in one
// `RideSession`, made when a ride is loaded (`beginRide`, called with the `ride_load` answer) and disposed when the
// next one begins. `useSession()` returns that one instance, so every component of a ride sees the same history.
import { useSyncExternalStore } from "react";
import type { RideLoaded } from "../bindings";

/** The live series a ride keeps a history of: the three traces (DESIGN §18.3). */
export type Series = "power" | "hr" | "cadence";

/** One measured sample at session second `t`, placed on the workout clock (display only: no derived physiology, #2). */
export interface Point {
  t: number;
  workoutS: number;
  v: number;
}

const EMPTY = { power: [], hr: [], cadence: [] } as const;

/** What one `ride_load` made. */
export class RideSession {
  /** The histories. Each is replaced on every append, never mutated, so a reader (and the render cache) sees the change. */
  series: Record<Series, readonly Point[]> = EMPTY;
  disposed = false;
  private lastT: Record<Series, number> = {
    power: -Infinity,
    hr: -Infinity,
    cadence: -Infinity,
  };
  private routed = false;

  constructor(readonly loaded: RideLoaded) {}

  get rideId(): string {
    return this.loaded.rideId;
  }
  /** `null` for a free ride. */
  get workoutId(): string | null {
    return this.loaded.workoutId;
  }

  /** The power history. */
  get power(): readonly Point[] {
    return this.series.power;
  }

  /** Record a sample of session time `t` once: a re-delivered sample (same `t`) is not a second point. */
  record(t: number, workoutS: number, v: number, of: Series = "power"): void {
    if (this.disposed || t <= this.lastT[of]) return;
    this.lastT[of] = t;
    this.series = {
      ...this.series,
      [of]: [...this.series[of], { t, workoutS, v }],
    };
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
    this.series = EMPTY;
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

/** The current session outside React (tests, event handlers). */
export const currentSession = (): RideSession | null => current;

/** The current ride's session, `null` until a ride is loaded. */
export const useSession = (): RideSession | null =>
  useSyncExternalStore(subscribe, () => current);

/** The ride's histories; re-renders on every appended sample. */
export const useSeries = (s: RideSession): Record<Series, readonly Point[]> =>
  useSyncExternalStore(subscribe, () => s.series);
