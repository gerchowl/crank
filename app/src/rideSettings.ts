// The ride-screen settings the type tiers read (Settings → Ride screen). The persistence primitive (#90) is not
// built yet, so these are constants behind one seam: when #90 lands, replace the bodies of the two functions
// below with reads from it and keep their signatures; nothing else in the tier code changes.
import { DEFAULT_DISTANCE_M } from "./typeTiers";

/** Rider distance in metres (#90 will persist it; until then the §6.2 default). */
export function useRideDistanceM(): number {
  return DEFAULT_DISTANCE_M;
}

/** Monitor physical-width override in mm (#90 will persist it; until then none). */
export function useMonitorWidthMmOverride(): number | null {
  return null;
}
