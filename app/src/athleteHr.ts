// The athlete's heart-rate references the ride's HR zone reads (DESIGN §18.3 "Heart-rate zone", #586): LTHR and HR
// max, bpm. The athlete model's estimates (spec/ONBOARDING.md §3, `hrMax` and `lthr`) and their manual overrides in
// Settings → Athlete model (#113, on the persistence primitive #90) are not built yet, so nothing in the app sets
// them: both are unknown, the ride shows no zone and Settings says why. #113 replaces the body of `useAthleteHr` with
// a read of the model and keeps its signature; the design fixtures and tests set the values through `setAthleteHr`.
import { useSyncExternalStore } from "react";
import type { HrRefs } from "./ride/hrZone";

const NONE: HrRefs = { lthrBpm: null, hrMaxBpm: null };
let refs: HrRefs = NONE;
const listeners = new Set<() => void>();

/** Sets the athlete's HR references (fixtures and tests until #113 reads them from the athlete model). */
export function setAthleteHr(next: Partial<HrRefs> | null): void {
  refs = next === null ? NONE : { ...NONE, ...next };
  for (const l of listeners) l();
}

/** The athlete's LTHR and HR max, bpm; each `null` while unknown. */
export function useAthleteHr(): HrRefs {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => refs,
  );
}
