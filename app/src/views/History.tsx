// History, MVP subset of #106 (DESIGN §5.5; #256 "the ride is saved, with a basic summary"): the ride list from
// `rides_list`, and one ride's summary from `ride_summary` at `history/<rideId>` (where the ride screen lands on
// `finished`). Only values the core returns are shown; there is no physiology math here (#89). Not here yet, by
// decision: intent verdicts, RPE, the coach timeline and ratings, the lap table, charts (A20) and analytics (A21).
import { useEffect } from "react";
import {
  commands,
  type ApiError,
  type RideOutcome,
  type RideRecord,
} from "../bindings";
import { navigate } from "../router";
import { useCommand } from "../stream";
import { formatDuration } from "./Workouts";

const OUTCOME: Record<RideOutcome, string> = {
  open: "Still recording",
  finished: "Finished",
  aborted: "Aborted",
};
const number = (n: number | null, unit: string, digits = 0) =>
  n === null || !Number.isFinite(n) ? "—" : `${n.toFixed(digits)} ${unit}`;
/** Work, one decimal below 10 kJ so a short ride is not rounded to 0 or 1. */
const work = (kj: number | null) =>
  number(kj, "kJ", kj !== null && Math.abs(kj) < 10 ? 1 : 0);
/** The ride's start in the rider's local time; "—" when the core has none. */
export function startedText(startedAt: number | null): string {
  if (startedAt === null) return "—";
  return new Date(startedAt * 1000).toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
}

function ErrorLine({
  error,
  onRetry,
}: {
  error: ApiError;
  onRetry: () => void;
}) {
  return (
    <p role="alert" className="history-error">
      {error.message}{" "}
      <button type="button" onClick={onRetry}>
        Retry
      </button>
    </p>
  );
}

function Leave() {
  return (
    <nav aria-label="Leave the summary" className="history-leave">
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

function SummaryBody({ ride }: { ride: RideRecord }) {
  const s = ride.summary;
  return (
    <section aria-label="Ride summary" className="history-summary">
      <p className="history-outcome" data-outcome={ride.outcome}>
        {OUTCOME[ride.outcome]}
      </p>
      <p className="muted">{startedText(ride.startedAt)}</p>
      {ride.outcome === "aborted" && (
        <p role="note" className="history-notice">
          Ride ended without the engine confirming; data may be incomplete.
        </p>
      )}
      {s === null ? (
        <p className="muted">No summary was recorded for this ride.</p>
      ) : (
        <>
          <dl>
            <dt>Duration</dt>
            <dd>{formatDuration(s.durationS)}</dd>
            <dt>Average power</dt>
            <dd>{number(s.avgPowerW, "W")}</dd>
            <dt>Normalized power</dt>
            <dd>{number(s.npW, "W")}</dd>
            <dt>Average heart rate</dt>
            <dd>{number(s.avgHrBpm, "bpm")}</dd>
            <dt>Work</dt>
            <dd>{work(s.kj)}</dd>
          </dl>
          {s.blocks.length > 0 && (
            <>
              <h3>Planned and ridden</h3>
              <table aria-label="Blocks">
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
                      <th scope="row">{b.index + 1}</th>
                      <td>
                        {formatDuration(b.plannedS)} · {number(b.plannedW, "W")}
                      </td>
                      <td>
                        {formatDuration(b.actualS)} · {number(b.actualW, "W")}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </>
          )}
        </>
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
    <div className="page history">
      <div className="page-head">
        <h1>{ride ? ride.workoutName : "Ride summary"}</h1>
      </div>
      {get.pending && !ride && <p className="muted">Loading…</p>}
      {error &&
        (error.code === "not_found" ? (
          <p role="alert" className="history-error">
            This ride isn’t in your history.
          </p>
        ) : (
          <ErrorLine error={error} onRetry={() => void run(id)} />
        ))}
      {ride && <SummaryBody ride={ride} />}
      <Leave />
    </div>
  );
}

function RideList() {
  const list = useCommand(commands.ridesList);
  const { run } = list;
  useEffect(() => {
    void run();
  }, [run]);
  return (
    <div className="page history">
      <div className="page-head">
        <h1>History</h1>
      </div>
      {list.pending && !list.data && <p className="muted">Loading…</p>}
      {list.error && (
        <ErrorLine error={list.error} onRetry={() => void run()} />
      )}
      {list.data?.length === 0 && <p className="muted">No rides yet.</p>}
      <ul className="history-list" aria-label="Rides">
        {list.data?.map((r) => (
          <li key={r.id}>
            <button type="button" onClick={() => navigate("history", r.id)}>
              <span className="history-name">{r.workoutName}</span>{" "}
              <span className="muted">
                {startedText(r.startedAt)} · {OUTCOME[r.outcome]}
                {r.summary?.durationS != null &&
                  ` · ${formatDuration(r.summary.durationS)}`}
              </span>
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}

/** `history` is the list; `history/<rideId>` is that ride's summary. */
export function History({ sub }: { sub?: string }) {
  return sub ? <RideSummaryView id={sub} /> : <RideList />;
}
