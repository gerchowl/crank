// History, MVP subset of #106 (DESIGN §5.5; #256 "the ride is saved, with a basic summary"): the ride list from
// `rides_list`, and one ride's Review from `ride_summary` at `history/<rideId>` (where the ride screen lands on
// `finished`). Only values the core returns are shown; there is no physiology math here (#89). Not here yet, by
// decision: intent verdicts, RPE, the coach timeline and ratings, the lap table, Evidence beyond the traces (#107) and
// analytics (#108); History's athlete model, CP trend and PD curve (#109).
// Direction A (§18, #529): the page title, then the list as a table of rides (date, ride, duration, work, outcome), each
// row with a 32 px Review button (§18.4); the ride last opened is marked when the rider comes back to the list.
// Review: the title with the way back, the outcome and start, then on the 12-column grid the recorded traces in 8
// columns (RideTraces.tsx, the ride session's history) and the summary figures in 4, then planned against ridden.
import { useEffect, useRef, useState } from "react";
import {
  commands,
  type ApiError,
  type RideOutcome,
  type RideRecord,
} from "../bindings";
import { navigate } from "../router";
import { useForcedEnd } from "../ride/forcedEnds";
import { useSeries, useSession, type RideSession } from "../ride/session";
import { useCommand } from "../stream";
import { RideTraces } from "./RideTraces";
import { formatDuration } from "./Workouts";
import "./history.css";

const OUTCOME: Record<RideOutcome, string> = {
  open: "Still recording",
  finished: "Finished",
  aborted: "Aborted",
};
/** `toFixed` without a negative zero: a value that rounds to nothing ("-0.0") is shown as plain zero. */
const fixed = (n: number, digits: number) => {
  const text = n.toFixed(digits);
  return Number(text) === 0 ? (0).toFixed(digits) : text;
};
const number = (n: number | null, unit: string, digits = 0) =>
  n === null || !Number.isFinite(n) ? "—" : `${fixed(n, digits)} ${unit}`;
/**
 * Work, one decimal while the value rounded to one decimal is below 10 kJ so a short ride is not rounded to 0 or 1,
 * whole kJ from there: the format follows the rounded value, so 9.95 reads "10 kJ", never "10.0 kJ".
 */
export const work = (kj: number | null) =>
  number(
    kj,
    "kJ",
    kj !== null && Math.round(Math.abs(kj) * 10) / 10 < 10 ? 1 : 0,
  );
/** The ride's start in the rider's local time; "—" when the core has none. */
export function startedText(startedAt: number | null): string {
  if (startedAt === null) return "—";
  return new Date(startedAt * 1000).toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
}

/** A failed command, as a notice: ⚠ and the core's message (§18.7 rule 11), and Retry where it helps. */
function ErrorLine({
  error,
  onRetry,
}: {
  error: ApiError | string;
  onRetry?: () => void;
}) {
  return (
    <div role="alert" className="history-notice" data-state="error">
      <p>
        <span data-state-label>⚠</span>{" "}
        {typeof error === "string" ? error : error.message}
      </p>
      {onRetry && (
        <button type="button" onClick={onRetry}>
          Retry
        </button>
      )}
    </div>
  );
}

function Leave() {
  return (
    <nav aria-label="Leave the summary" className="review-leave">
      <button type="button" onClick={() => navigate("workouts")}>
        Back to Workouts
      </button>
      <button type="button" onClick={() => navigate("today")}>
        Today
      </button>
      <button type="button" onClick={() => navigate("history")}>
        All rides
      </button>
    </nav>
  );
}

/** The traces of the ride this window rode, from its session; re-renders while that ride still records. */
function Recorded({
  session,
  durationS,
}: {
  session: RideSession;
  durationS: number | null;
}) {
  const series = useSeries(session);
  return <RideTraces series={series} durationS={durationS} />;
}

function SummaryBody({ ride }: { ride: RideRecord }) {
  const s = ride.summary;
  const forced = useForcedEnd(ride.id);
  const session = useSession();
  // Only this window's own ride has its traces: the session that rode it, until the next load replaces it.
  const recorded = session?.rideId === ride.id ? session : null;
  return (
    <section aria-label="Ride summary" className="review-body">
      <p className="review-meta">
        <span
          className="review-outcome"
          data-outcome={ride.outcome}
          data-state={ride.outcome}
        >
          <span data-state-label>{OUTCOME[ride.outcome]}</span>
        </span>
        <span className="num">{startedText(ride.startedAt)}</span>
      </p>
      {forced && (
        <p role="note" className="history-notice history-forced">
          Ride ended without the engine confirming; data may be incomplete.
        </p>
      )}
      <div className="review-grid">
        <div className="review-panel review-main">
          <h2>Traces</h2>
          {recorded ? (
            <Recorded session={recorded} durationS={s?.durationS ?? null} />
          ) : (
            <p className="history-hint">
              Traces show for the ride you have just ridden, until you load the
              next one. Earlier rides show their figures only.
            </p>
          )}
        </div>
        <div className="review-panel review-side">
          <h2>Summary</h2>
          {s === null ? (
            <p className="history-hint">
              No summary was recorded for this ride.
            </p>
          ) : (
            <dl className="review-stats">
              <div>
                <dt>Duration</dt>
                <dd className="num">{formatDuration(s.durationS)}</dd>
              </div>
              <div>
                <dt>Work</dt>
                <dd className="num">{work(s.kj)}</dd>
              </div>
              <div>
                <dt>Average power</dt>
                <dd className="num">{number(s.avgPowerW, "W")}</dd>
              </div>
              <div>
                <dt>Normalized power</dt>
                <dd className="num">{number(s.npW, "W")}</dd>
              </div>
              <div>
                <dt>Average heart rate</dt>
                <dd className="num">{number(s.avgHrBpm, "bpm")}</dd>
              </div>
            </dl>
          )}
        </div>
      </div>
      {s !== null && s.blocks.length > 0 && (
        <div className="review-panel">
          <h2>Planned and ridden</h2>
          <table aria-label="Blocks" className="review-blocks">
            <thead>
              <tr>
                <th scope="col">Block</th>
                <th scope="col">Planned</th>
                <th scope="col">Ridden</th>
              </tr>
            </thead>
            <tbody>
              {s.blocks.map((b) => (
                <tr key={b.index}>
                  <th scope="row" className="num">
                    {b.index + 1}
                  </th>
                  <td className="num">
                    {formatDuration(b.plannedS)} · {number(b.plannedW, "W")}
                  </td>
                  <td className="num">
                    {formatDuration(b.actualS)} · {number(b.actualW, "W")}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}

export function RideSummaryView({ id }: { id: string }) {
  const get = useCommand(commands.rideSummary);
  const { run } = get;
  useEffect(() => {
    void run(id);
  }, [run, id]);
  const ride = get.data?.id === id ? get.data : null;
  // The hook keeps the previous call's failure until the next call finishes; one for another ride, or a
  // retry in flight, is not shown next to "Loading…".
  const error = get.pending ? null : get.error;
  return (
    <div className="page history review">
      <div className="review-head">
        <h1>{ride ? ride.workoutName : "Ride summary"}</h1>
        <Leave />
      </div>
      {get.pending && !ride && <p className="history-hint">Loading…</p>}
      {error &&
        (error.code === "not_found" ? (
          <ErrorLine error="This ride isn’t in your history." />
        ) : (
          <ErrorLine error={error} onRetry={() => void run(id)} />
        ))}
      {ride && <SummaryBody ride={ride} />}
    </div>
  );
}

/** One ride's row: its cells named by the column heads (a table, not a list), its Review button by everything that
 * tells the ride apart, so the row reads the same to a screen reader as to the eye. */
function RideRow({ ride: r, last }: { ride: RideRecord; last: boolean }) {
  const started = startedText(r.startedAt);
  const duration = formatDuration(r.summary?.durationS ?? null);
  return (
    <>
      <span role="cell" className="num">
        {started}
      </span>
      <span role="rowheader" className="history-name">
        <span>{r.workoutName}</span>
        {last && <span className="history-tag">Last opened</span>}
      </span>
      <span role="cell" className="num">
        {duration}
      </span>
      <span role="cell" className="num">
        {work(r.summary?.kj ?? null)}
      </span>
      <span role="cell" data-state={r.outcome}>
        <span data-state-label>{OUTCOME[r.outcome]}</span>
      </span>
      <span role="cell">
        <button
          type="button"
          // The visible word first (WCAG 2.5.3), then what tells this ride apart from the next.
          aria-label={`Review ${r.workoutName}, ${started}, ${OUTCOME[r.outcome]}, ${duration}`}
          onClick={() => navigate("history", r.id)}
        >
          Review
        </button>
      </span>
    </>
  );
}

function RideList({ lastOpened }: { lastOpened: string | null }) {
  const list = useCommand(commands.ridesList);
  const { run } = list;
  useEffect(() => {
    void run();
  }, [run]);
  // The hook keeps the last failure until the next call finishes; a Retry in flight does not show it.
  const error = list.pending ? null : list.error;
  const rides = list.data ?? [];
  // The ride last opened stays in view when the rider comes back to a long list.
  const marked = useRef<HTMLDivElement>(null);
  const hasMarked = rides.some((r) => r.id === lastOpened);
  useEffect(() => {
    if (hasMarked) marked.current?.scrollIntoView?.({ block: "nearest" });
  }, [hasMarked]);
  return (
    <div className="page history">
      <h1>History</h1>
      {list.pending && !list.data && <p className="history-hint">Loading…</p>}
      {error && <ErrorLine error={error} onRetry={() => void run()} />}
      {list.data?.length === 0 && (
        <div className="history-empty">
          <p>No rides yet.</p>
          <p className="history-hint">
            Every ride you start is saved here, with its summary.
          </p>
        </div>
      )}
      {rides.length > 0 && (
        <div role="table" aria-label="Rides" className="history-table">
          <div role="rowgroup" className="history-rows">
            <div role="row" className="history-columns">
              <span role="columnheader">Started</span>
              <span role="columnheader">Ride</span>
              <span role="columnheader">Duration</span>
              <span role="columnheader">Work</span>
              <span role="columnheader">Outcome</span>
              <span role="columnheader">
                <span className="sr-only">Review</span>
              </span>
            </div>
          </div>
          <div role="rowgroup" className="history-rows history-list">
            {rides.map((r) => {
              const last = r.id === lastOpened;
              return (
                <div
                  role="row"
                  key={r.id}
                  ref={last ? marked : undefined}
                  className="history-row"
                  aria-current={last || undefined}
                >
                  <RideRow ride={r} last={last} />
                </div>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}

/**
 * `history` is the list; `history/<rideId>` is that ride's Review. History holds the ride whose Review was open last,
 * so the list marks it on the way back; it lives as long as History is the view (App keys the view by its name), so
 * leaving History for another view forgets it.
 */
export function History({ sub }: { sub?: string }) {
  const [lastOpened, setLastOpened] = useState<string | null>(sub ?? null);
  // Set while rendering, not in an effect: the list's first render already marks the ride.
  if (sub !== undefined && sub !== lastOpened) setLastOpened(sub);
  return sub ? (
    <RideSummaryView id={sub} />
  ) : (
    <RideList lastOpened={lastOpened} />
  );
}
