// Workouts library, MVP subset of #110 (DESIGN §5.7): the list from the core, a pick shows the compiled summary
// (duration range, TSS, IF, zone time, issues), and "Ride this" loads the ride and routes to it. Not here yet,
// by decision (#256): the editor pane and AI diff (A24/A25), search and tags, and the health-stop guard.
import { useCallback, useEffect, useState } from "react";
import {
  commands,
  type ApiError,
  type CompiledView,
  type WorkoutIssue,
} from "../bindings";
import { useCommand } from "../stream";
import { dismissRideLoad, startRideLoad, useRideLoad } from "./rideLoad";

/** `m:ss`, or `h:mm:ss` from an hour; "—" for an unknown duration. */
export function formatDuration(s: number | null): string {
  if (s === null || !Number.isFinite(s) || s < 0) return "—";
  const t = Math.round(s);
  const h = Math.floor(t / 3600);
  const m = Math.floor((t % 3600) / 60);
  const sec = String(t % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${sec}` : `${m}:${sec}`;
}
/** The duration, as a range when the compiled workout has open-ended steps. */
export function durationText(c: CompiledView): string {
  const [lo, hi] = c.totalRangeS;
  return lo !== null && hi !== null && lo !== hi
    ? `${formatDuration(lo)}–${formatDuration(hi)}`
    : formatDuration(c.totalS);
}
const number = (n: number | null, digits: number) =>
  n === null || !Number.isFinite(n) ? "—" : n.toFixed(digits);

function ErrorLine({
  error,
  onRetry,
}: {
  error: ApiError;
  onRetry?: () => void;
}) {
  return (
    <p role="alert" className="workouts-error">
      {error.message}
      {error.code === "device_unavailable" &&
        ". Pair or check your trainer in Settings → Devices."}{" "}
      {onRetry && (
        <button type="button" onClick={onRetry}>
          Retry
        </button>
      )}
    </p>
  );
}

function Issues({ issues }: { issues: WorkoutIssue[] }) {
  if (issues.length === 0) return null;
  return (
    <ul className="workouts-issues" aria-label="Issues">
      {issues.map((i, n) => (
        <li
          key={`${i.kind}-${i.nodeId ?? "all"}-${n}`}
          data-severity={i.severity}
        >
          <strong>{i.severity === "error" ? "Error" : "Warning"}</strong>{" "}
          {i.message}
        </li>
      ))}
    </ul>
  );
}

function Summary({ compiled }: { compiled: CompiledView }) {
  return (
    <section aria-label="Compiled summary" className="workouts-summary">
      <dl>
        <dt>Duration</dt>
        <dd>{durationText(compiled)}</dd>
        <dt>TSS</dt>
        <dd>{number(compiled.tss, 0)}</dd>
        <dt>IF</dt>
        <dd>{number(compiled.if, 2)}</dd>
      </dl>
      <h3>Time in zone</h3>
      <ul aria-label="Time in zone">
        {compiled.timeInZoneS.map((s, i) => (
          <li key={i}>
            <span>Z{i + 1}</span> {formatDuration(s)}
          </li>
        ))}
      </ul>
      <Issues issues={compiled.issues} />
    </section>
  );
}

/** How often the core is asked whether a load is still in flight, while it says one is. */
const STATUS_POLL_MS = 400;

export function Workouts() {
  const list = useCommand(commands.workoutsList);
  const get = useCommand(commands.workoutGet);
  const status = useCommand(commands.rideStatus);
  const load = useRideLoad();
  const [picked, setPicked] = useState<string | null>(null);
  const { run: runList } = list;
  const { run: runGet } = get;
  const { run: runStatus } = status;
  useEffect(() => {
    void runList();
  }, [runList]);

  // The selection is locked while a ride load is in flight. The core can replace a loaded-but-unstarted ride
  // (`ride_load` stops and replaces it, core.rs:761-783; `ride_end` ends one, :1011), but it refuses a second
  // load DURING the first ("another ride is loading or loaded", :789-792), and the page must never show a
  // workout other than the one being loaded: success would route into the old workout's ride, a refusal would
  // appear under the new one (#320). "In flight" is not this view's state: the app-scope controller (rideLoad.ts)
  // knows a load this window started, and `ride_status().loading` (core.rs:1089) is the core's own answer for one
  // that started anywhere (a window reloaded mid-load), asked when the view mounts and until it clears.
  const coreLoading = status.data?.loading === true;
  useEffect(() => {
    void runStatus();
  }, [runStatus]);
  useEffect(() => {
    if (!coreLoading) return;
    const t = setTimeout(() => void runStatus(), STATUS_POLL_MS);
    return () => clearTimeout(t);
  }, [coreLoading, status.data, runStatus]);
  const loading = load.phase === "loading" || coreLoading;
  // A refusal is shown where the rider is looking (#333), whether it arrived while this view was mounted or while
  // they were elsewhere (then on return). The notice hides on this view, and leaving it marks the refusal seen
  // (`RideLoadNotice`, app scope, so StrictMode's mount/unmount probe cannot clear it), so it is shown once here and never reappears elsewhere. It stays until the rider picks
  // another workout, dismisses it or leaves; under its workout when that one is picked, else at the top by name.
  const refusal = load.phase === "refused" ? load : null;

  const pick = useCallback(
    (id: string) => {
      setPicked(id);
      dismissRideLoad();
      void runGet(id);
    },
    [runGet],
  );
  /**
   * The one seam for the start guard. When the health-stop guard lands (A6 `requestStart`, #81) it replaces
   * this call: `requestStart(workout, "workouts")` decides, and loads through `ride_load` itself. There is no
   * guard today and none is faked. The load itself is owned at app scope (`startRideLoad`): it begins the ride
   * session (`beginRide`) and routes to the ride only if the rider is still here when it completes.
   */
  const rideThis = useCallback(
    (id: string, name: string) => void startRideLoad(id, name),
    [],
  );
  // A refusal belongs to the workout it was for; for any other it names the workout.
  const here = refusal !== null && refusal.workoutId === picked;

  const detail = picked !== null ? get.data : null;
  // The core refuses a workout with an error-severity issue (`ride_load`: invalid input); say so up front.
  const blocking =
    detail?.compiled.issues.find((i) => i.severity === "error") ?? null;
  return (
    <div className="page workouts">
      <div className="page-head">
        <h1>Workouts</h1>
      </div>
      {list.error && (
        <ErrorLine error={list.error} onRetry={() => void runList()} />
      )}
      {refusal && !here && (
        <p role="alert" className="workouts-error">
          Could not load {refusal.name}: {refusal.error.message}{" "}
          <button type="button" onClick={dismissRideLoad}>
            Dismiss
          </button>
        </p>
      )}
      <div className="workouts-body">
        <nav aria-label="Library">
          {loading && (
            <p role="status" className="muted">
              Loading ride…
            </p>
          )}
          {list.data?.length === 0 && <p className="muted">No workouts yet.</p>}
          <ul className="workouts-list">
            {list.data?.map((w) => (
              <li key={w.id}>
                <button
                  type="button"
                  aria-pressed={picked === w.id}
                  disabled={loading}
                  onClick={() => pick(w.id)}
                >
                  <span className="workouts-name">{w.name}</span>{" "}
                  <span className="muted">
                    {formatDuration(w.totalS)} · TSS {number(w.tss, 0)}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </nav>
        <div className="workouts-detail">
          {picked !== null && get.pending && <p className="muted">Loading…</p>}
          {picked !== null && get.error && <ErrorLine error={get.error} />}
          {detail && !get.pending && (
            <>
              <h2>{detail.workout.name}</h2>
              <Summary compiled={detail.compiled} />
              <button
                type="button"
                className="primary"
                disabled={loading || blocking !== null}
                aria-describedby={blocking ? "ride-blocked" : undefined}
                onClick={() => rideThis(detail.workout.id, detail.workout.name)}
              >
                Ride this
              </button>
              {blocking && (
                <p id="ride-blocked" className="muted">
                  Can’t ride this until the error is fixed: {blocking.message}
                </p>
              )}
              {refusal && here && <ErrorLine error={refusal.error} />}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
