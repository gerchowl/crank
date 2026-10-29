// The non-modal notice for a ride load that finished while the rider was elsewhere (#320): they are not pulled out of
// what they are doing, and the result is one click away. On the Workouts view the view itself shows a refusal under
// its workout; on the ride screen there is nothing left to say.
import { useEffect } from "react";
import { useRoute } from "../router";
import { dismissRideLoad, goToRide, useRideLoad } from "./rideLoad";

export function RideLoadNotice() {
  const load = useRideLoad();
  const { view } = useRoute();
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
