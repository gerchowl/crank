// The non-modal notice for a ride load that finished while the rider was elsewhere (#320): they are not pulled out of
// what they are doing, and the result is one click away. On the Workouts view the view itself takes a refusal (#333),
// so it is shown once there and cleared. On the ride screen the notice is silent: a loaded result is dismissed on entry
// (the rider is where it leads), and a refusal that arrives there waits for the next view (see rideLoad.ts, #338).
import { useEffect, useRef } from "react";
import { useRoute } from "../router";
import {
  dismissRideLoad,
  goToRide,
  seenRideRefusal,
  useRideLoad,
} from "./rideLoad";

export function RideLoadNotice() {
  const load = useRideLoad();
  const { view } = useRoute();
  // Leaving Workouts marks a refusal it showed as seen (#333): once there, never again elsewhere.
  const previous = useRef(view);
  useEffect(() => {
    if (previous.current === "workouts" && view !== "workouts")
      seenRideRefusal();
    previous.current = view;
  }, [view]);
  useEffect(() => {
    if (view === "ride") dismissRideLoad();
  }, [view]);
  if (load.phase === "loaded" && view !== "ride")
    return (
      <p role="status" className="ride-notice">
        Ride loaded: {load.name}{" "}
        <button type="button" onClick={goToRide}>
          Go to ride
        </button>{" "}
        <button type="button" onClick={dismissRideLoad}>
          Dismiss
        </button>
      </p>
    );
  if (load.phase === "refused" && view !== "workouts" && view !== "ride")
    return (
      <p role="alert" className="ride-notice">
        Could not load {load.name}: {load.error.message}{" "}
        <button type="button" onClick={dismissRideLoad}>
          Dismiss
        </button>
      </p>
    );
  return null;
}
