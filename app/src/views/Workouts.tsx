// Workouts library, MVP subset of #110 (DESIGN §5.7): the list from the core, a pick shows the compiled summary
// (duration range, TSS, IF, zone time, issues), and "Ride this" loads the ride and routes to it. Not here yet,
// by decision (#256): the editor pane and AI diff (A24/A25), search and tags, and the health-stop guard.
// Direction A (§18, #528): the page title, then on the 12-column grid (§18.2) the library in 7 columns and the picked
// workout in one panel in 5. The library is a single-select listbox whose selection follows focus (WAI-ARIA APG): a row
// is a workout of any name length, not a 32 px button (§18.4), and Up/Down/Home/End walk it. The panel puts "Ride this"
// right under the workout's profile and figures, so it stays in the window however long the block list runs.
import { Play } from "lucide-react";
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import {
  commands,
  type ApiError,
  type CompiledView,
  type WorkoutBlock,
  type WorkoutIssue,
} from "../bindings";
import { PHASE_NAME, watts } from "../ride/Timeline";
import { useCommand } from "../stream";
import { dismissRideLoad, startRideLoad, useRideLoad } from "./rideLoad";
import "./workouts.css";

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
const icon = { size: 16, strokeWidth: 1.5, "aria-hidden": true } as const;

/** A failed command, as a notice: ⚠ and the core's message (§18.7 rule 11), what to do, and Retry where it helps. */
function ErrorLine({
  error,
  onRetry,
}: {
  error: ApiError;
  onRetry?: () => void;
}) {
  return (
    <div role="alert" className="workouts-notice" data-state="error">
      <p>
        <span data-state-label>⚠</span> {error.message}
        {error.code === "device_unavailable" &&
          ". Pair or check your trainer in Settings → Devices."}
      </p>
      {onRetry && (
        <button type="button" onClick={onRetry}>
          Retry
        </button>
      )}
    </div>
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
          data-state={i.severity}
        >
          <span data-state-label>⚠</span>
          <span>
            <strong>{i.severity === "error" ? "Error" : "Warning"}</strong>{" "}
            {i.message}
          </span>
        </li>
      ))}
    </ul>
  );
}

const PW = 400;
const PH = 80;
/** The workout's blocks by height over its length (P13), a ramp as a slope; the block list below gives the words. */
function Profile({
  blocks,
  totalS,
}: {
  blocks: readonly WorkoutBlock[];
  totalS: number | null;
}) {
  const shown = blocks.filter((b) => b.startS !== null && b.durS !== null);
  if (shown.length === 0) return null;
  const xMax = Math.max(
    totalS ?? 0,
    ...shown.map((b) => (b.startS ?? 0) + (b.durS ?? 0)),
    1,
  );
  const top = Math.max(
    1,
    ...shown.map((b) => Math.max(b.powerFrom ?? 0, b.powerTo ?? 0)),
  );
  const x = (s: number) => (s / xMax) * PW;
  const y = (w: number) => PH - (w / top) * PH * 0.95;
  return (
    // eslint-disable-next-line no-restricted-syntax -- a data graphic, not an icon (DESIGN §18.7 rule 4 is about icons).
    <svg
      data-graphic
      className="workouts-profile"
      role="img"
      aria-label={`Profile: ${shown.length} blocks over ${formatDuration(xMax)}`}
      viewBox={`0 0 ${PW} ${PH}`}
      preserveAspectRatio="none"
    >
      {shown.map((b, i) => {
        const [s, e] = [b.startS ?? 0, (b.startS ?? 0) + (b.durS ?? 0)];
        const [from, to] = [b.powerFrom ?? 0, b.powerTo ?? b.powerFrom ?? 0];
        return (
          <path
            key={i}
            className="workouts-profile-block"
            data-phase={b.phase}
            d={`M${x(s) + 0.5},${PH} L${x(s) + 0.5},${y(from)} L${x(e) - 0.5},${y(to)} L${x(e) - 0.5},${PH} Z`}
            vectorEffect="non-scaling-stroke"
          />
        );
      })}
    </svg>
  );
}

/** Every compiled block in order: its phase, its length and its watts. */
function Blocks({ blocks }: { blocks: readonly WorkoutBlock[] }) {
  if (blocks.length === 0) return null;
  return (
    <section className="workouts-part" aria-labelledby="workouts-blocks-h">
      <h3 id="workouts-blocks-h">Blocks</h3>
      <ol className="workouts-blocks">
        {blocks.map((b, i) => (
          <li key={`${b.nodeId}-${b.iteration.join(".")}-${i}`}>
            <span className="num workouts-index">{i + 1}</span>
            <span>{PHASE_NAME[b.phase]}</span>
            <span className="num">
              {b.durRangeS?.[0] != null && b.durRangeS[1] != null
                ? `${formatDuration(b.durRangeS[0])}–${formatDuration(b.durRangeS[1])}`
                : formatDuration(b.durS)}
            </span>
            <span className="num">{watts(b) || "—"}</span>
          </li>
        ))}
      </ol>
    </section>
  );
}

function Summary({ compiled }: { compiled: CompiledView }) {
  const most = Math.max(1, ...compiled.timeInZoneS);
  return (
    <section aria-label="Compiled summary" className="workouts-summary">
      <dl className="workouts-stats">
        <div>
          <dt>Duration</dt>
          <dd className="num">{durationText(compiled)}</dd>
        </div>
        <div>
          <dt>TSS</dt>
          <dd className="num">{number(compiled.tss, 0)}</dd>
        </div>
        <div>
          <dt>IF</dt>
          <dd className="num">{number(compiled.if, 2)}</dd>
        </div>
      </dl>
      <Issues issues={compiled.issues} />
      <section className="workouts-part" aria-labelledby="workouts-zones-h">
        <h3 id="workouts-zones-h">Time in zone</h3>
        <ul className="workouts-zones" aria-label="Time in zone">
          {compiled.timeInZoneS.map((s, i) => (
            <li key={i}>
              <span>Z{i + 1}</span>
              <span className="workouts-zone-track" aria-hidden>
                <span
                  className="workouts-zone-bar"
                  data-zone={i + 1}
                  style={{ width: `${(s / most) * 100}%` }}
                />
              </span>
              <span className="num">{formatDuration(s)}</span>
            </li>
          ))}
        </ul>
      </section>
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
  // (`ride_load` stops and replaces it, core.rs `ride_load` (~1679); `ride_end` ends one), but it refuses a second
  // load DURING the first ("another ride is loading or loaded", core.rs `ride_load` (~1679)), and the page must never show a
  // workout other than the one being loaded: success would route into the old workout's ride, a refusal would
  // appear under the new one (#320). "In flight" is not this view's state: the app-scope controller (rideLoad.ts)
  // knows a load this window started, and `ride_status().loading` (core.rs `ride_status` (~2382)) is the core's own answer for one
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

  // The listbox (APG, selection follows focus): one tab stop, the picked row or else the first; Up/Down/Home/End move
  // focus and pick. While a load is in flight every row is disabled (#320) and nothing moves.
  const workouts = list.data ?? [];
  const rows = useRef(new Map<string, HTMLLIElement>());
  const stop = workouts.some((w) => w.id === picked)
    ? picked
    : (workouts[0]?.id ?? null);
  const onKey = (e: KeyboardEvent<HTMLUListElement>) => {
    if (loading || workouts.length === 0) return;
    const at = workouts.findIndex((w) => w.id === stop);
    const to =
      e.key === "ArrowDown"
        ? Math.min(at + 1, workouts.length - 1)
        : e.key === "ArrowUp"
          ? Math.max(at - 1, 0)
          : e.key === "Home"
            ? 0
            : e.key === "End"
              ? workouts.length - 1
              : null;
    if (to === null) return;
    e.preventDefault();
    const id = workouts[to].id;
    rows.current.get(id)?.focus();
    if (id !== picked) pick(id);
  };
  const choose = (id: string) => {
    if (!loading && id !== picked) pick(id);
  };

  const detail = picked !== null ? get.data : null;
  // The core refuses a workout with an error-severity issue (`ride_load`: invalid input); say so up front.
  const blocking =
    detail?.compiled.issues.find((i) => i.severity === "error") ?? null;
  const busy = load.phase === "loading" && load.workoutId === picked;
  return (
    <div className="page workouts">
      <h1>Workouts</h1>
      {list.error && (
        <ErrorLine error={list.error} onRetry={() => void runList()} />
      )}
      {refusal && !here && (
        <div role="alert" className="workouts-notice" data-state="error">
          <p>
            <span data-state-label>⚠</span> Could not load {refusal.name}:{" "}
            {refusal.error.message}
          </p>
          <button type="button" onClick={dismissRideLoad}>
            Dismiss
          </button>
        </div>
      )}
      <div className="workouts-body">
        <div className="workouts-library">
          {loading && (
            <p role="status" className="workouts-hint">
              Loading ride…
            </p>
          )}
          {list.data?.length === 0 && (
            <p className="workouts-hint">
              No workouts yet. The built-in workouts and the ones you write will
              be listed here.
            </p>
          )}
          {workouts.length > 0 && (
            <div className="workouts-table">
              <div className="workouts-columns" aria-hidden>
                <span>Name</span>
                <span>Duration</span>
                <span>TSS</span>
              </div>
              <ul
                role="listbox"
                aria-label="Library"
                aria-disabled={loading || undefined}
                className="workouts-list"
                onKeyDown={onKey}
              >
                {workouts.map((w) => (
                  <li
                    key={w.id}
                    ref={(el) => {
                      if (el) rows.current.set(w.id, el);
                      else rows.current.delete(w.id);
                    }}
                    role="option"
                    aria-selected={picked === w.id}
                    aria-disabled={loading || undefined}
                    tabIndex={stop === w.id ? 0 : -1}
                    onClick={() => choose(w.id)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault();
                        choose(w.id);
                      }
                    }}
                  >
                    <span className="workouts-name">{w.name}</span>
                    <span className="num">
                      <span className="sr-only">, </span>
                      {formatDuration(w.totalS)}
                    </span>
                    <span className="num">
                      <span className="sr-only">, TSS </span>
                      {number(w.tss, 0)}
                    </span>
                  </li>
                ))}
              </ul>
            </div>
          )}
        </div>
        <div className="workouts-detail">
          {picked === null && list.data && list.data.length > 0 && (
            <p className="workouts-hint">
              Pick a workout to see its profile, its figures and its blocks.
            </p>
          )}
          {picked !== null && get.pending && (
            <p className="workouts-hint">Loading…</p>
          )}
          {picked !== null && get.error && <ErrorLine error={get.error} />}
          {detail && !get.pending && (
            <>
              <div className="workouts-detail-head">
                <h2>{detail.workout.name}</h2>
                {detail.workout.ftp !== null && (
                  <span className="workouts-chip num">
                    FTP {detail.workout.ftp} W
                  </span>
                )}
              </div>
              <Profile
                blocks={detail.compiled.blocks}
                totalS={detail.compiled.totalS}
              />
              <div className="workouts-actions">
                <button
                  type="button"
                  className="primary"
                  disabled={loading || blocking !== null}
                  aria-busy={busy || undefined}
                  aria-describedby={blocking ? "ride-blocked" : undefined}
                  onClick={() =>
                    rideThis(detail.workout.id, detail.workout.name)
                  }
                >
                  <Play {...icon} />
                  Ride this
                </button>
                {blocking && (
                  <p id="ride-blocked" className="workouts-blocked">
                    Can’t ride this until the error is fixed: {blocking.message}
                  </p>
                )}
              </div>
              {refusal && here && <ErrorLine error={refusal.error} />}
              <Summary compiled={detail.compiled} />
              <Blocks blocks={detail.compiled.blocks} />
            </>
          )}
        </div>
      </div>
    </div>
  );
}
