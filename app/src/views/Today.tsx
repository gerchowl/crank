// Today, direction A (DESIGN §5.1 "In direction A", §16.2, §18; #530): the home screen, on what the core can answer
// today. The page title and the date, then on the 12-column grid (§18.2) four panels: today's ride (7 columns) beside
// the devices (5), the last 7 days (7) beside the last ride (5). The days show minutes ridden, not §5.1's TSS: the stored
// summary has none yet (#570). Not here yet, because the contract has none of it
// (#29): the check-in and red-flag check, the planner's proposal with its reasons, the wish matrix and the health cap
// (#94, #119, #86); the athlete model card (#18, #109). The key hints are drawn; binding them is #104, as on the ride.
// Today's ride is the ride load the rider started in Workouts (rideLoad.ts): while it runs Start reads "Loading ride…",
// once loaded Start leads to the Ready sheet (§5.4); a refusal stays the notice's (RideLoadNotice, unchanged).
import { Bluetooth, Heart, History as HistoryIcon, Play } from "lucide-react";
import { useEffect, useState } from "react";
import {
  commands,
  type ApiError,
  type DeviceRole,
  type DeviceView,
  type RideRecord,
} from "../bindings";
import { LinkChip, LinkHints, RetryLinks } from "../links/chips";
import { navigate } from "../router";
import { useCommand } from "../stream";
import { useUi, useUiClient } from "../stream/uiHooks";
import { OUTCOME, startedText, work } from "./History";
import { goToRide, useRideLoad } from "./rideLoad";
import { formatDuration } from "./Workouts";
import "./today.css";

const icon = { size: 16, strokeWidth: 1.5, "aria-hidden": true } as const;

/** A failed read, as a notice: ⚠ and the core's message (§18.7 rule 11), and Retry where it helps. */
function ErrorLine({
  error,
  onRetry,
}: {
  error: ApiError;
  onRetry?: () => void;
}) {
  return (
    <div role="alert" className="today-notice" data-state="error">
      <p>
        <span data-state-label>⚠</span> {error.message}
      </p>
      {onRetry && (
        <button type="button" onClick={onRetry}>
          Retry
        </button>
      )}
    </div>
  );
}

/** Today's ride: the load started in Workouts, or nothing planned and the way to pick one. */
function TodaysRide() {
  const load = useRideLoad();
  const loading = load.phase === "loading";
  const loaded = load.phase === "loaded";
  return (
    <section aria-labelledby="today-ride" className="today-panel today-wide">
      <h2 id="today-ride">Today’s ride</h2>
      {loading || loaded ? (
        <>
          <p className="today-ride-name">{load.name}</p>
          <p className="today-hint">
            {loading
              ? "Connecting your devices and loading the ride."
              : "Loaded. Start opens the ready check, where the ride begins."}
          </p>
        </>
      ) : (
        <>
          <p className="today-ride-name">Nothing planned</p>
          <p className="today-hint">
            Pick a workout and ride it; it shows here, ready to start.
          </p>
        </>
      )}
      <div className="today-actions">
        {loading || loaded ? (
          <button
            type="button"
            className="primary"
            disabled={loading}
            aria-busy={loading || undefined}
            onClick={goToRide}
          >
            <Play {...icon} />
            {loading ? "Loading ride…" : "Start"}
            {loaded && <kbd aria-hidden>⏎</kbd>}
          </button>
        ) : (
          <button
            type="button"
            className="primary"
            onClick={() => navigate("workouts")}
          >
            Pick a workout
          </button>
        )}
      </div>
    </section>
  );
}

const ROLE = {
  trainer: { label: "Trainer", Icon: Bluetooth, chip: "trainer" },
  heart_rate: { label: "Heart-rate strap", Icon: Heart, chip: "strap" },
} as const;

/**
 * One role: who is paired, and its link chip (links/chips.tsx): the keeper's phase, or the ride's link while a ride
 * holds it (the frame's `devices`, ADR-0001 §4.3.6, ADR-0002 §5.1.4).
 */
function DeviceRow({ of: role, info }: { of: DeviceRole; info: DeviceView }) {
  const { label, Icon, chip } = ROLE[role];
  const d = info.paired;
  return (
    <li
      className="today-device"
      data-state={d ? "paired" : role === "trainer" ? "missing" : "unpaired"}
    >
      <Icon {...icon} />
      <span className="today-device-role">{label}</span>
      <span className="today-device-name">{d ? (d.name ?? d.id) : "—"}</span>
      <span className="today-device-state">
        {d ? (
          <LinkChip of={chip} device={info} />
        ) : (
          <>
            <span data-state-label>{role === "trainer" ? "⚠" : "–"}</span> not
            paired
          </>
        )}
      </span>
    </li>
  );
}

/**
 * The paired devices, from the snapshot channel (a pairing, a forget or a load shows in its next frame). Until the
 * first frame they are loading; a channel that cannot open says why (`ui_open`: the pairings cannot be read), and the
 * channel's watchdog opens it again every 2 s, Retry at once.
 */
function Devices() {
  const ui = useUi();
  const client = useUiClient();
  const devices = ui.devices;
  const error = devices === null ? ui.error : null;
  const noTrainer = devices !== null && devices.trainer.paired === null;
  return (
    <section aria-labelledby="today-devices" className="today-panel">
      <h2 id="today-devices">Devices</h2>
      {devices === null && error === null && (
        <p className="today-hint">Loading…</p>
      )}
      {error && <ErrorLine error={error} onRetry={() => client.reopen()} />}
      {devices && (
        <ul className="today-devices" aria-label="Paired devices">
          <DeviceRow of="trainer" info={devices.trainer} />
          <DeviceRow of="heart_rate" info={devices.strap} />
        </ul>
      )}
      {devices && <LinkHints devices={devices} />}
      {noTrainer && (
        <p className="today-hint" data-state="warning">
          <span data-state-label>⚠</span> A ride needs a paired trainer. Pair it
          in Settings → Devices.
        </p>
      )}
      <div className="today-actions">
        <button
          type="button"
          className={noTrainer ? "primary" : undefined}
          onClick={() => navigate("settings", "devices")}
        >
          <Bluetooth {...icon} />
          {noTrainer ? "Pair a trainer" : "Devices"}
        </button>
        {devices && <RetryLinks devices={devices} />}
      </div>
    </section>
  );
}

/** The local calendar day of a Unix-seconds instant, as the Unix seconds of its local midnight. */
const dayOf = (s: number) => {
  const d = new Date(s * 1000);
  d.setHours(0, 0, 0, 0);
  return d.getTime() / 1000;
};
/** The seven local days ending today, oldest first, each with the rides started on it. */
export function lastSevenDays(rides: RideRecord[], nowMs: number) {
  const today = dayOf(nowMs / 1000);
  const days = Array.from({ length: 7 }, (_, i) => {
    // Step by calendar day, not by 86 400 s: a daylight-saving change makes one day 23 or 25 hours long.
    const d = new Date(today * 1000);
    d.setDate(d.getDate() - (6 - i));
    return { start: d.getTime() / 1000, rides: [] as RideRecord[] };
  });
  for (const r of rides) {
    if (r.startedAt === null) continue;
    const day = days.find((d) => d.start === dayOf(r.startedAt ?? 0));
    day?.rides.push(r);
  }
  return days.map((d, i) => {
    const timed = d.rides.filter((r) => r.summary !== null);
    return {
      start: d.start,
      today: i === 6,
      rides: d.rides.length,
      /**
       * Minutes ridden by the day's rides with a summary; `null` when none has one. §5.1 wants TSS here, which the
       * stored summary does not carry yet (dto.rs `RideSummary`, #570).
       */
      minutes:
        timed.length === 0
          ? null
          : Math.round(
              timed.reduce((sum, r) => sum + (r.summary?.durationS ?? 0), 0) /
                60,
            ),
    };
  });
}

function Week({ rides, now }: { rides: RideRecord[]; now: number }) {
  const days = lastSevenDays(rides, now);
  return (
    <ol className="today-week" aria-label="Last 7 days">
      {days.map((d) => {
        const date = new Date(d.start * 1000);
        const state = d.rides > 0 ? "ridden" : d.today ? "today" : "rest";
        return (
          <li
            key={d.start}
            className="today-day"
            data-state={state}
            aria-current={d.today ? "date" : undefined}
          >
            <span className="today-day-name">
              {d.today
                ? "Today"
                : date.toLocaleDateString(undefined, { weekday: "short" })}
            </span>
            <span className="today-day-date num">
              {date.toLocaleDateString(undefined, {
                day: "numeric",
                month: "short",
              })}
            </span>
            <span className="today-day-value num">
              {d.minutes === null ? (
                // A dash alone is skipped or read as "em dash": say what it stands for.
                <>
                  <span aria-hidden>—</span>
                  <span className="sr-only">
                    {d.rides === 0 ? "no ride" : "no time recorded"}
                  </span>
                </>
              ) : (
                <>
                  {d.minutes}
                  <span className="today-unit"> min</span>
                </>
              )}
            </span>
            <span className="today-day-state" data-state-label>
              {state === "ridden"
                ? d.rides === 1
                  ? "1 ride"
                  : `${d.rides} rides`
                : state === "today"
                  ? "no ride yet"
                  : "rest"}
            </span>
          </li>
        );
      })}
    </ol>
  );
}

/** The newest stored ride with its figures and the way to its Review. */
function LastRide({ ride: r }: { ride: RideRecord }) {
  const started = startedText(r.startedAt);
  const duration = formatDuration(r.summary?.durationS ?? null);
  return (
    <>
      <p className="today-ride-name">{r.workoutName}</p>
      <dl className="today-figures">
        <div>
          <dt>Started</dt>
          <dd className="num">{started}</dd>
        </div>
        <div>
          <dt>Duration</dt>
          <dd className="num">{duration}</dd>
        </div>
        <div>
          <dt>Work</dt>
          <dd className="num">{work(r.summary?.kj ?? null)}</dd>
        </div>
        <div>
          <dt>Outcome</dt>
          <dd data-state={r.outcome}>
            <span data-state-label>{OUTCOME[r.outcome]}</span>
          </dd>
        </div>
      </dl>
      <div className="today-actions">
        <button
          type="button"
          // The visible word first (WCAG 2.5.3), then what tells this ride apart, as History's row does.
          aria-label={`Review ${r.workoutName}, ${started}, ${OUTCOME[r.outcome]}, ${duration}`}
          onClick={() => navigate("history", r.id)}
        >
          <HistoryIcon {...icon} />
          Review
        </button>
      </div>
    </>
  );
}

function Rides({ now }: { now: number }) {
  const list = useCommand(commands.ridesList);
  const { run } = list;
  useEffect(() => {
    void run();
  }, [run]);
  const error = list.pending ? null : list.error;
  const rides = list.data;
  // `rides_list` is newest first (core.rs `Core::rides_list`), so the last ride is the first.
  const last = rides?.[0];
  const loadingHint = list.pending && !rides && (
    <p className="today-hint">Loading…</p>
  );
  return (
    <>
      <section aria-labelledby="today-week" className="today-panel today-wide">
        <h2 id="today-week">Last 7 days</h2>
        {loadingHint}
        {error && <p className="today-hint">Your rides could not be read.</p>}
        {rides && <Week rides={rides} now={now} />}
      </section>
      <section aria-labelledby="today-last" className="today-panel">
        <h2 id="today-last">Last ride</h2>
        {loadingHint}
        {error && <ErrorLine error={error} onRetry={() => void run()} />}
        {rides?.length === 0 && (
          <p className="today-hint">
            No rides yet. Every ride you start is saved in History.
          </p>
        )}
        {last && <LastRide ride={last} />}
      </section>
    </>
  );
}

export function Today() {
  // "Today" is the day the view opened: the shell mounts it afresh on every visit.
  const [now] = useState(() => Date.now());
  return (
    <div className="page today">
      <div className="today-head">
        <h1>Today</h1>
        <p className="today-date">
          {new Date(now).toLocaleDateString(undefined, {
            weekday: "long",
            day: "numeric",
            month: "long",
          })}
        </p>
      </div>
      <div className="today-grid">
        <TodaysRide />
        <Devices />
        <Rides now={now} />
      </div>
    </div>
  );
}
