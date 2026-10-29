// The rides the core ended by force, as `ride_end` told us (`RideEnded.forced`, #331/#334). The fact exists only
// on that answer: the stored record says `aborted` for a forced end but also for any other abort, so the summary
// keys its note on this set, never on `outcome`. The `finished` status frame can route to the summary before the
// answer arrives, so readers subscribe (`useForcedEnd`). It lives for the window; a reload loses it (nothing
// persists it: the core's `RideRecord` has no such field).
import { useSyncExternalStore } from "react";

let forced: ReadonlySet<string> = new Set();
const listeners = new Set<() => void>();
const set = (next: ReadonlySet<string>) => {
  forced = next;
  for (const l of [...listeners]) l();
};

/** Remember that the core ended this ride by force. */
export const markForcedEnd = (rideId: string): void =>
  set(new Set(forced).add(rideId));
/** Did the core end this ride by force (in this window)? */
export const wasForcedEnd = (rideId: string): boolean => forced.has(rideId);
/** For tests: forget every forced end. */
export const resetForcedEnds = (): void => set(new Set());
/** `wasForcedEnd`, re-rendering when the answer arrives after the route. */
export const useForcedEnd = (rideId: string): boolean =>
  useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => {
        listeners.delete(l);
      };
    },
    () => forced.has(rideId),
  );
