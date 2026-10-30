// Ride (DESIGN §5.2, §6.1, §6.4, §16.1; #96 MVP subset, #256): ride the loaded workout in ERG with live tiles, the
// whole-ride overview and pause / skip / end. It renders what the stream and the core's answers say and branches
// on `ApiError.code`, never on `message` (ADR-0001 §4.4); the core's text is shown as the reason of a refusal.
//
// Loss and reconnect states (#100, DESIGN §6.5/§6.6) are read from the status and the links only, never inferred: the
// loss fields #329 added (`trainerLostSinceS`, `reconnectAttempt`, `softStartTargetW`, `lastGapS`, `reconnectedAtS`,
// `userPaused`) are reported only while the ride is Active, and every duration is a difference on the session clock
// (`sessionS`, which the heartbeat refreshes), never a timer of this screen.
// Out of this subset, each with its issue: density modes, the coach lane, the road (#96), sound and the input
// dispatcher, the rest of the failure states (#100), the Review/summary
// (#106: a finished ride routes to History with the ride id until it exists) and the phone layout.
// A window reload loses the session; the core keeps the ride loaded, and picking it up again is the Resume sheet (#259).
import { useEffect, useState } from "react";
import "./ride.css";
import { commands } from "../bindings";
import type {
  ApiError,
  CompiledView,
  LinkStatus,
  StreamStatus,
  WorkoutBlock,
} from "../bindings";
import { navigate } from "../router";
import {
  StreamProvider,
  useCommand,
  useLinks,
  useStatus,
  useStreamError,
} from "../stream";
import { useSubscribe } from "../stream";
import { RideTiers } from "../useTypeTiers";
import { markForcedEnd } from "./forcedEnds";
import { RideChart } from "./RideChart";
import { releaseRide, usePower, useSession, type RideSession } from "./session";

/** `m:ss`, `h:mm:ss` from an hour; `—` while unknown. */
export function clock(s: number | null): string {
  if (s === null) return "—";
  const t = Math.max(0, Math.floor(s));
  const [h, m, sec] = [Math.floor(t / 3600), Math.floor(t / 60) % 60, t % 60];
  const ss = String(sec).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

/** The planned power now: the block's `powerFrom → powerTo` at its elapsed time (the core compiled the line). */
export function targetNow(
  block: WorkoutBlock | undefined,
  elapsedS: number | null,
): number | null {
  if (!block || block.powerFrom === null) return null;
  const to = block.powerTo ?? block.powerFrom;
  if (!block.durS || elapsedS === null) return block.powerFrom;
  const f = Math.min(1, Math.max(0, elapsedS / block.durS));
  return Math.round(block.powerFrom + (to - block.powerFrom) * f);
}

/**
 * The TARGET tile: what the engine commanded the trainer into (#324), as a word or a number. No arithmetic: the
 * watts are the engine's, and why there is no ERG target comes from its own flags (`ergReleased`, `userPaused`).
 */
export function commandedView(st: StreamStatus | null): {
  value: number | string | null;
  unit: string;
} {
  const c = st?.commanded;
  if (!c) return { value: null, unit: "W" };
  switch (c.kind) {
    case "erg":
      return { value: c.watts, unit: "W" };
    case "resistance":
      // The rider's pause outranks a release (policy order, erg/policy.rs; #330 hides the release while paused). With
      // neither flag this is the status Resume publishes at once, while `last_sent` is still the pause's
      // `Resistance { 0 }`; the next tick sends ERG (≤ 250 ms). Publishing after the resumed command (#324 review
      // option a) was rejected on purpose: the mode comes from the policy, which runs once per tick with the tick's
      // cadence, flow and block clock (one SetMode per tick, ADR-0002 §4.4; compliance and release counters advance in
      // it), so a command-time policy run would double-count a tick and fork the tick-aligned following machine; and
      // delaying Resume's publish would leave `userPaused` stale for a tick. There is no target to show, so "—".
      return {
        value: st.userPaused ? "Paused" : st.ergReleased ? "Released" : null,
        unit: "",
      };
    case "free":
      return { value: "Free", unit: "" };
    case "sim":
      return { value: "Sim", unit: "" };
  }
}

const PHASE_WORD = {
  warmup: "WARM-UP",
  work: "WORK",
  recovery: "RECOVER",
  cooldown: "COOL-DOWN",
  free: "FREE",
} as const;

/** What the state line says; every word is the core's status, none is guessed. */
function stateLine(st: StreamStatus | null): string {
  if (!st) return "Connecting to the ride…";
  if (st.recordError !== null) return `Not recording: ${st.recordError}`;
  const s = st.state;
  switch (s.kind) {
    case "idle":
      return "Loaded. Press Start; the trainer waits for you to pedal.";
    case "finished":
      return "Finished";
    case "paused":
      return s.reason === "user"
        ? "Paused"
        : s.reason === "trainer_lost" && st.trainerLostSinceS !== null
          ? "Paused: trainer lost"
          : "Paused: waiting for you to pedal";
    case "riding":
      return st.ergReleased ? "ERG released: spin up to re-engage" : "Riding";
  }
}

/** DESIGN §5.2: "Trainer lost ≥ 30 s" adds what to try and End…, measured on the session clock. */
export const LONG_LOSS_S = 30;
/** The threshold for the factual "Back after m:ss" row (DESIGN §5.2); the §6.6 re-entry policy itself waits on #50. */
export const RE_ENTRY_S = 120;
/** DESIGN §6.5: the "NEO back · m:ss gap" chip stays this long after the reconnect. */
export const BACK_CHIP_S = 10;

/**
 * The sensor-loss rows of DESIGN §6.5/§6.6, from the status and the links only. The trainer is lost when
 * `trainerLostSinceS` is set (not when `state` says `trainer_lost`, which stays after the trainer is back until the
 * rider pedals, and not when a tile happens to be empty); durations are `sessionS` minus a stamp, so they are unknown
 * (`null`) rather than guessed while `sessionS` is. The strap is lost when its link is `lost` in a live ride, ERG is
 * released when the core says so in a riding ride. Guards that the core already guarantees stay as defence, not as
 * logic: the loss fields are null outside Active (so `live &&` changes nothing today), `sessionS` never goes backwards
 * (so `now - backAt >= 0` holds), and a stamp with no clock yields no duration.
 */
export function lossView(
  st: StreamStatus | null,
  links: { trainer?: LinkStatus; hr?: LinkStatus },
) {
  const kind = st?.state.kind;
  const live = kind === "riding" || kind === "paused";
  const since = live ? (st?.trainerLostSinceS ?? null) : null;
  const now = st?.sessionS ?? null;
  const gap = live ? (st?.lastGapS ?? null) : null;
  const backAt = live ? (st?.reconnectedAtS ?? null) : null;
  const trainer = since !== null;
  const lostForS = since !== null && now !== null ? now - since : null;
  return {
    trainer,
    // `trainerLostSinceS` is also set after 8 ticks without samples on a link that is still ready: "reconnecting" is
    // only said while the link is lost or connecting (no links frame yet: unknown, neutral wording).
    reconnecting: links.trainer === "lost" || links.trainer === "connecting",
    attempt: trainer ? (st?.reconnectAttempt ?? null) : null,
    lostForS,
    long: lostForS !== null && lostForS >= LONG_LOSS_S,
    // The trainer is back, the rider is not pedalling yet: `state` is still `paused(trainer_lost)`.
    reEntry:
      !trainer &&
      gap !== null &&
      gap >= RE_ENTRY_S &&
      st?.state.kind === "paused" &&
      st.state.reason === "trainer_lost",
    gapS: gap,
    softStartW: live && !trainer ? (st?.softStartTargetW ?? null) : null,
    backChip:
      !trainer &&
      gap !== null &&
      backAt !== null &&
      now !== null &&
      now - backAt >= 0 &&
      now - backAt < BACK_CHIP_S,
    hr: live && links.hr === "lost",
    erg: kind === "riding" && st?.ergReleased === true,
  };
}

const REFUSAL: Record<string, string> = {
  conflict: "Not possible right now",
  device_unavailable: "Trainer not ready",
  storage: "The ride could not be recorded",
  internal: "The ride engine stopped",
  engine_unresponsive: "The ride engine is not answering",
};

/** One value of the grid: its slot reserves the widest form (`ch`), so a longer number moves nothing (P4, §16.1). */
function Tile({
  id,
  label,
  unit,
  ch,
  value,
  sub,
}: {
  id: string;
  label: string;
  unit: string;
  ch: number;
  value: number | string | null;
  /** A smaller line under the value, in a slot of its own that reserves its widest form. */
  sub?: { id: string; ch: number; text: string };
}) {
  return (
    <div className="tile" data-tile={id}>
      <span className="tile-label">{label}</span>
      <span className="tile-row">
        <span
          className="tile-value num"
          data-slot={id}
          style={{ width: `${ch}ch` }}
        >
          {value ?? "—"}
        </span>
        <span className="tile-unit">{unit}</span>
      </span>
      {sub && (
        <span
          className="tile-sub num"
          data-slot={sub.id}
          style={{ width: `${sub.ch}ch` }}
        >
          {sub.text}
        </span>
      )}
    </div>
  );
}

function Pill({ name, link }: { name: string; link: LinkStatus | undefined }) {
  const word = link === undefined ? "not connected" : link;
  return (
    <span className="pill-half" data-link={link ?? "none"}>
      {link === "lost" ? "⚠ " : ""}
      {name} <span aria-hidden>●</span>
      <span className="sr-only"> {word}</span>
    </span>
  );
}

function useDetail(session: RideSession): CompiledView | null {
  const [detail, setDetail] = useState<CompiledView | null>(null);
  const id = session.workoutId;
  useEffect(() => {
    if (id === null) return;
    let live = true;
    void commands.workoutGet(id).then((r) => {
      if (live && r.status === "ok") setDetail(r.data.compiled);
    });
    return () => {
      live = false;
    };
  }, [id]);
  return detail;
}

function Screen({ session }: { session: RideSession }) {
  const status = useStatus();
  const links = useLinks();
  // A wedged engine (#351): the stream's commands are refused after the core's bound, so the values below are stale.
  const unresponsive = useStreamError()?.code === "engine_unresponsive";
  const power = useSubscribe("power", { rateHz: 4 }).sample;
  const cadence = useSubscribe("cadence", { rateHz: 2 }).sample;
  const hr = useSubscribe("hr", { rateHz: 2 }).sample;
  const compiled = useDetail(session);
  const history = usePower(session);

  const start = useCommand(commands.rideStart);
  const pause = useCommand(commands.ridePause);
  const resume = useCommand(commands.rideResume);
  const skip = useCommand(commands.rideSkip);
  const end = useCommand(commands.rideEnd);
  const [refusal, setRefusal] = useState<ApiError | null>(null);
  const [confirming, setConfirming] = useState<"end" | "skip" | null>(null);
  // Guards against a second tap before the core has answered in its status (no timers): Start until the ride leaves
  // `idle`, Skip until the block index has moved past the one that was skipped.
  const [startSent, setStartSent] = useState(false);
  // Pause and Resume are two buttons in two places, and the one just tapped stays disabled until the status shows
  // its effect, so a double tap cannot become pause-then-resume.
  const [pauseSent, setPauseSent] = useState<"pause" | "resume" | null>(null);
  const loss = lossView(status, links);
  const [skippedFrom, setSkippedFrom] = useState<number | null>(null);

  const workoutS = status?.workoutS ?? null;
  const kind = status?.state.kind ?? "idle";
  const finished = kind === "finished";
  useEffect(() => {
    if (finished) session.leaveOnce(() => navigate("history", session.rideId));
  }, [finished, session]);
  // The measured power joins the history on the workout clock; a gap (`v` null) or a frozen clock adds nothing.
  const [sampleT, sampleV] = [power?.t ?? null, power?.v ?? null];
  useEffect(() => {
    if (sampleT !== null && sampleV !== null && workoutS !== null)
      session.record(sampleT, workoutS, sampleV);
  }, [session, sampleT, sampleV, workoutS]);

  const run = async <T,>(
    go: () => Promise<
      { status: "ok"; data: T } | { status: "error"; error: ApiError }
    >,
  ) => {
    setRefusal(null);
    const outcome = await go();
    if (outcome.status === "error") setRefusal(outcome.error);
    return outcome;
  };
  const startRide = async () => {
    setStartSent(true);
    const outcome = await run(start.run);
    if (outcome.status === "error") setStartSent(false);
  };
  const pauseOrResume = async (which: "pause" | "resume") => {
    setPauseSent(which);
    const outcome = await run(which === "pause" ? pause.run : resume.run);
    if (outcome.status === "error") setPauseSent(null);
  };
  const skipBlock = async () => {
    setConfirming(null);
    setSkippedFrom(blockIdx);
    const outcome = await run(skip.run);
    if (outcome.status === "error") setSkippedFrom(null);
  };
  const endRide = async () => {
    setConfirming(null);
    const outcome = await run(end.run);
    // The summary says so when the core ended the ride by force (`forced`, not the recorded outcome). Marked
    // before leaving, and also when the `finished` frame already routed: the summary follows the mark.
    if (outcome.status === "ok" && outcome.data.forced && outcome.data.rideId)
      markForcedEnd(outcome.data.rideId);
    if (outcome.status === "ok")
      session.leaveOnce(() => {
        if (outcome.data.rideId !== null) {
          navigate("history", outcome.data.rideId);
        } else {
          // Never started: unloaded, nothing to review.
          releaseRide(session);
          navigate("workouts");
        }
      });
  };

  const blocks = compiled?.blocks ?? [];
  const blockIdx = status?.blockIndex ?? null;
  const block = blockIdx === null ? undefined : blocks[blockIdx];
  const target = targetNow(block, status?.blockElapsedS ?? null);
  const targetView = commandedView(status);
  const blockLeft =
    block?.durS != null && status?.blockElapsedS != null
      ? block.durS - status.blockElapsedS
      : (status?.blockElapsedS ?? null);
  const total = compiled?.totalS ?? null;
  // End is never held by a pending command: the rider always has a way out. The core does not bound the ride commands
  // it relays to the engine (`EngineHandle::send` waits on the engine's reply, engine.rs `EngineHandle::send` (~226)), and a second `ride_end`
  // while one is running is not an error but waits for the same end (core.rs `end_ride` (~1742), bounded by REAP_TIMEOUT 10 s
  // and RELEASE_TIMEOUT 3 s, core.rs `REAP_TIMEOUT` (~55)), so End may be tapped again, also while an earlier End has not answered.
  // The rider's pause is the core's `userPaused`, not the pause reason: under a trainer loss `state` says
  // `trainer_lost` while the core still holds the rider pause (#327 item 3).
  const userPaused = status?.userPaused === true;
  // The tap's guard ends when the status shows its effect, whoever caused it (the coach may resume a rider pause).
  if (
    (pauseSent === "pause" && userPaused) ||
    (pauseSent === "resume" && !userPaused)
  )
    setPauseSent(null);
  const busy =
    start.pending ||
    pause.pending ||
    resume.pending ||
    skip.pending ||
    end.pending;
  // Skip needs a next block to skip: none for a free ride (the core refuses, engine.rs `command` `EngineCmd::SkipBlock` (~1347)) and none known until the
  // plan has arrived. A skip in the last block ends the ride (the engine finishes it): it asks like End.
  const canSkip = blocks.length > 0 && blockIdx !== null;
  const skipEnds = blocks.length > 0 && blockIdx === blocks.length - 1;
  const skipWaits = skippedFrom !== null && skippedFrom === blockIdx;
  const live = kind === "riding" || kind === "paused";
  const canPause = live && !userPaused && !(pauseSent === "pause");
  const canResume = userPaused && pauseSent !== "resume";
  // Trainer lost: the core records `null` for power and cadence (DESIGN §6.5), so their tiles say "—" whatever the last
  // sample was; HR lost blanks the HR tile.
  const round = (v: number | null | undefined, blank = false) =>
    v == null || blank ? null : Math.round(v);

  return (
    <div className="ride" data-testid="ride">
      <header className="ride-bar">
        <div className="ride-controls">
          <button
            className="primary ride-btn"
            disabled={busy || status === null || kind !== "idle" || startSent}
            onClick={() => void startRide()}
          >
            Start
          </button>
          <button
            className="ride-btn"
            disabled={busy || !canPause}
            title={userPaused ? "Already paused" : undefined}
            onClick={() => void pauseOrResume("pause")}
          >
            Pause
          </button>
          <button
            className="ride-btn"
            disabled={busy || !canResume}
            onClick={() => void pauseOrResume("resume")}
          >
            Resume
          </button>
          <button
            className="ride-btn"
            disabled={busy || !live || !canSkip || skipWaits}
            onClick={() =>
              skipEnds ? setConfirming("skip") : void skipBlock()
            }
          >
            Skip block
          </button>
          <button
            className="ride-btn"
            disabled={finished || status === null}
            onClick={() => setConfirming("end")}
          >
            End
          </button>
        </div>
        <div className="ride-times num" data-testid="times">
          <span data-slot="elapsed" style={{ width: "7ch" }}>
            {clock(workoutS)}
          </span>
          <span className="muted"> ridden · </span>
          <span data-slot="left" style={{ width: "7ch" }}>
            {total === null || workoutS === null
              ? "—"
              : clock(total - workoutS)}
          </span>
          <span className="muted"> left</span>
        </div>
        <span
          className="ride-chip"
          data-slot="neo-back"
          data-testid="neo-back"
          style={{ width: "20ch" }}
        >
          {loss.backChip ? `NEO back · ${clock(loss.gapS)} gap` : ""}
        </span>
        <div className="ride-pill" aria-label="Devices">
          <Pill name="NEO" link={links.trainer} />
          <span aria-hidden> │ </span>
          <Pill name="HR" link={links.hr} />
        </div>
      </header>

      <section className="ride-hero" aria-label="Block">
        <div className="hero-row1">
          <span data-slot="block">
            {block
              ? `${PHASE_WORD[block.phase]} · block ${(blockIdx ?? 0) + 1} of ${blocks.length}`
              : blocks.length > 0 && blockIdx === null
                ? "Last block done"
                : session.workoutId === null
                  ? "FREE RIDE"
                  : "Loading the plan…"}
          </span>
          <span className="muted">
            {block?.durS == null ? "in block" : "left in block"}
          </span>
        </div>
        <div className="hero-t1 num" data-slot="countdown">
          {clock(blockLeft)}
        </div>
        <div className="hero-band">
          <Tile
            id="target"
            label="TARGET"
            unit={targetView.unit}
            ch={8}
            value={targetView.value}
            sub={{
              id: "plan",
              ch: 10,
              text: target === null ? "Plan —" : `Plan ${target} W`,
            }}
          />
          <Tile
            id="power"
            label="POWER"
            unit="W"
            ch={4}
            value={round(power?.v, loss.trainer)}
          />
        </div>
      </section>

      <aside className="ride-side" aria-label="Body">
        <Tile
          id="hr"
          label="HEART RATE"
          unit="bpm"
          ch={3}
          value={round(hr?.v, loss.hr)}
        />
        <Tile
          id="cadence"
          label="CADENCE"
          unit="rpm"
          ch={3}
          value={round(cadence?.v, loss.trainer)}
        />
        <div className="coach-cell" data-cell="coach" aria-hidden />
      </aside>

      <div className="ride-notice">
        <p className="ride-state" data-testid="state">
          {stateLine(status)}
        </p>
        {loss.trainer && (
          <p role="status" className="ride-loss" data-testid="loss-trainer">
            <strong>
              {loss.reconnecting
                ? loss.hr
                  ? "NEO and HR lost"
                  : "NEO lost"
                : loss.hr
                  ? "No data from the NEO · HR lost"
                  : "No data from the NEO"}
            </strong>{" "}
            {loss.lostForS !== null && (
              <>
                <strong data-testid="lost-for">
                  {clock(loss.lostForS)}
                </strong>{" "}
              </>
            )}
            · workout paused at {clock(status?.blockElapsedS ?? null)} of this
            block
            {loss.reconnecting
              ? loss.attempt === null
                ? " · reconnecting"
                : ` · reconnecting (attempt ${loss.attempt})`
              : ""}{" "}
            · keep pedalling or stop — your ride is saved
          </p>
        )}
        {loss.trainer && loss.long && (
          <p role="status" className="ride-loss" data-testid="loss-help">
            <button className="ride-end" onClick={() => setConfirming("end")}>
              End…
            </button>{" "}
            Try: power-cycle NEO · close Zwift
          </p>
        )}
        {/* Only the fact (`lastGapS`). DESIGN §6.6's policy text (re-warm-up, restarting the interval, "no resume" past
            15 min) is not shown: the engine applies none of it (`recover()` eases to the block's own target and the block
            resumes where it froze) and the status carries no re-entry plan until #50 lands. */}
        {loss.reEntry && (
          <p role="status" className="ride-loss" data-testid="re-entry">
            <strong>↻ Back after {clock(loss.gapS)}</strong>
          </p>
        )}
        {loss.softStartW !== null && (
          <p role="status" className="ride-loss" data-testid="soft-start">
            <strong>Reconnected</strong> · easing back in to{" "}
            {Math.round(loss.softStartW)} W
          </p>
        )}
        {loss.hr && !loss.trainer && (
          <p role="status" className="ride-loss" data-testid="loss-hr">
            <strong>HR lost</strong> · heart rate shows — · the ride goes on
          </p>
        )}
        {loss.erg && (
          <p role="status" className="ride-loss" data-testid="loss-erg">
            <strong>ERG released</strong> — spin up to 60 rpm
          </p>
        )}
        {unresponsive && (
          <p
            role="alert"
            className="ride-loss"
            data-testid="engine-unresponsive"
          >
            <strong>The ride engine is not answering</strong> — the values are
            not live · End still ends the ride
          </p>
        )}
        {refusal && (
          <p role="alert" className="ride-refusal" data-code={refusal.code}>
            <strong>{REFUSAL[refusal.code] ?? "Failed"}:</strong>{" "}
            {refusal.message}
          </p>
        )}
      </div>

      <footer className="ride-overview">
        <RideChart
          blocks={blocks}
          totalS={total}
          power={history}
          workoutS={workoutS ?? 0}
        />
      </footer>

      {confirming && (
        <div className="scrim">
          <div
            role="alertdialog"
            aria-modal="true"
            aria-labelledby="end-title"
            className="confirm"
            onKeyDown={(e) => {
              if (e.key === "Escape") setConfirming(null);
            }}
          >
            <h2 id="end-title">End this ride?</h2>
            <p>
              {confirming === "skip"
                ? "This is the last block: skipping it ends the ride, saved up to now."
                : kind === "idle"
                  ? "The ride has not started; it will be unloaded."
                  : "The ride is saved up to now and cannot be resumed."}
            </p>
            <button autoFocus onClick={() => setConfirming(null)}>
              Keep riding
            </button>
            <button
              className="primary"
              onClick={() =>
                void (confirming === "skip" ? skipBlock() : endRide())
              }
            >
              End ride
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

export function Ride() {
  const session = useSession();
  if (session === null)
    return (
      <div className="page">
        <div className="page-head">
          <h1>No ride loaded</h1>
          <p className="muted">
            Pick a workout in Workouts and choose Ride this.
          </p>
          <button onClick={() => navigate("workouts")}>Go to Workouts</button>
        </div>
      </div>
    );
  return (
    <div className="ride-root">
      <StreamProvider key={session.rideId}>
        <RideTiers>
          <Screen session={session} />
        </RideTiers>
      </StreamProvider>
    </div>
  );
}
