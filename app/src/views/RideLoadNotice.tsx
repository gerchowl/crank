// The non-modal notice for a ride load that finished while the rider was elsewhere (#320): they are not pulled out of
// what they are doing, and the result is one click away. On the Workouts view the view itself takes a refusal (#333),
// so it is shown once there and cleared. On the ride screen the notice is silent: a loaded result is dismissed on entry
// (the rider is where it leads), and a refusal that arrives there waits for the next view (see rideLoad.ts, #338).
// It sits at the top of the main region, above the view, in the page's flow (App.tsx `Destination`): it covers nothing
// and is the first thing read and tabbed to; ⚠ or ✓ and a word carry its state (DESIGN §18.7 rule 11), and its buttons
// are the 32 px family (§18.4).
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
  // Today shows a loaded ride as today's ride, with Start (Today.tsx), so the notice would say it twice there.
  if (load.phase === "loaded" && view !== "ride" && view !== "today")
    return (
      <div className="ride-notice-band">
        <p role="status" className="ride-notice" data-state="loaded">
          <span className="ride-notice-text">
            <span data-state-label>✓</span> Ride loaded: {load.name}
          </span>
          <span className="ride-notice-actions">
            <button type="button" className="primary" onClick={goToRide}>
              Go to ride
            </button>
            <button type="button" onClick={dismissRideLoad}>
              Dismiss
            </button>
          </span>
        </p>
      </div>
    );
  if (load.phase === "refused" && view !== "workouts" && view !== "ride")
    return (
      <div className="ride-notice-band">
        <p role="alert" className="ride-notice" data-state="error">
          <span className="ride-notice-text">
            <span data-state-label>⚠</span> Could not load {load.name}:{" "}
            {load.error.message}
          </span>
          <span className="ride-notice-actions">
            <button type="button" onClick={dismissRideLoad}>
              Dismiss
            </button>
          </span>
        </p>
      </div>
    );
  return null;
}
