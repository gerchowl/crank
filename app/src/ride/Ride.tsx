// Ride (DESIGN §5.2, §6.1, §16.1, §18.2–§18.5 direction A; #96 MVP subset, #256, #525): ride the loaded workout in ERG.
// The anatomy is fixed (§18.2): status bar, the traces column (banner band, power / heart rate / cadence traces) beside
// the metric column (countdown and next, power, heart rate, cadence), the block timeline and the key row; each region
// carries `data-region`, and the design harness checks its box is the same in every state (rule 10, #498). It renders
// what the stream and the core's answers say and branches on `ApiError.code`, never on `message` (ADR-0001 §4.4); the
// core's text is shown as the reason of a refusal. The key hints are labels: the bindings come with the dispatcher (#104).
//
// Loss and reconnect states (#100, DESIGN §6.5/§6.6) are read from the status and the links only, never inferred: the
// loss fields #329 added (`trainerLostSinceS`, `reconnectAttempt`, `softStartTargetW`, `lastGapS`, `reconnectedAtS`,
// `userPaused`) are reported only while the ride is Active, and every duration is a difference on the session clock
// (`sessionS`, which the heartbeat refreshes), never a timer of this screen.
// Out of this subset, each with its issue: density modes, the coach lane (#99) and its coach marks on the timeline,
// phase templates and the ERG band (#97), sound and the input dispatcher (#104), the symptom stop (#103), the rest of
// the failure states (#100), the Review/summary
// (#106: a finished ride routes to History with the ride id until it exists) and the phone layout.
// A window reload loses the session; the core keeps the ride loaded, and picking it up again is the Resume sheet (#259).
import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import {
  Bluetooth,
  Heart,
  Keyboard,
  Pause,
  Play,
  RotateCw,
  SkipForward,
  Square,
  Timer,
  Zap,
} from "lucide-react";
import "./ride.css";
import { commands } from "../bindings";
import type {
  ApiError,
  LinkStatus,
  StreamFollowing,
  StreamStatus,
  WorkoutBlock,
  WorkoutDetail,
} from "../bindings";
import { Num } from "../Num";
import { navigate } from "../router";
import {
  StreamProvider,
  useCommand,
  useLinks,
  useStatus,
  useStreamError,
} from "../stream";
import { useSubscribe } from "../stream";
import { openKeys } from "../shell/keys";
import { BannerBand, banners, trainerNow } from "./banners";
import { useFit } from "./fit";
import { markForcedEnd } from "./forcedEnds";
import {
  releaseRide,
  useSeries,
  useSession,
  type RideSession,
  type Series,
} from "./session";
import {
  ReadyBody,
  readyBanner,
  readyView,
  type RideCommand,
} from "./ReadySheet";
import { PHASE_NAME, Timeline, watts } from "./Timeline";
import { Trace } from "./Trace";

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
      // The rider's pause outranks a release (policy order, erg/policy.rs `ErgPolicy::desired` (~172); #330 hides the release while paused). With
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

/**
 * What the state line says once the ride has started; every word is the core's status, none is guessed. Before the
 * start (no status yet, or idle) the line is the Ready sheet's (`ReadyView.line`).
 */
function stateLine(st: StreamStatus | null): string {
  if (!st) return "";
  if (st.recordError !== null) return `Not recording: ${st.recordError}`;
  const s = st.state;
  switch (s.kind) {
    case "idle":
      return "";
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

const icon = { size: 16, strokeWidth: 1.5, "aria-hidden": true } as const;

/** The status-bar chrome above and the timeline, key row and main padding below the main row (DESIGN §18.2). */
export const CHROME_PX = 236;
/** The metric column's minimum content (§18.2 budget): a shorter main row cannot hold the tiers (§6.2 too-small rule). */
export const MAIN_MIN_PX = 462;
const TOO_SMALL = "Window too small to read targets";
const onResize = (l: () => void) => {
  window.addEventListener("resize", l);
  return () => window.removeEventListener("resize", l);
};
const useTooSmall = () =>
  useSyncExternalStore(
    onResize,
    () => window.innerHeight - CHROME_PX < MAIN_MIN_PX,
  );

/** A key hint inside a button (§18.4); hidden from the accessible name, which is the action's word. */
const Key = ({ k }: { k: string }) => <kbd aria-hidden>{k}</kbd>;

/** A live value in a slot that reserves its widest form (`ch`), so a longer number moves nothing (P4, §16.1). */
function Value({
  id,
  ch,
  value,
  tier,
}: {
  id: string;
  ch: number;
  value: number | string | null;
  tier?: "t2";
}) {
  return (
    <Num data-slot={id} data-tier={tier} data-live style={{ width: `${ch}ch` }}>
      {value ?? "—"}
    </Num>
  );
}

function Pill({
  name,
  link,
  Icon,
}: {
  name: string;
  link: LinkStatus | undefined;
  Icon: typeof Heart;
}) {
  const word = link === undefined ? "not connected" : link;
  return (
    <span
      className="pill-half"
      data-link={link ?? "none"}
      data-state={link ?? "none"}
    >
      <Icon {...icon} />
      {link === "lost" ? "⚠ " : ""}
      {name} <span aria-hidden>●</span>
      <span className="sr-only" data-state-label>
        {" "}
        {word}
      </span>
    </span>
  );
}

function useDetail(session: RideSession): WorkoutDetail | null {
  const [detail, setDetail] = useState<WorkoutDetail | null>(null);
  const id = session.workoutId;
  useEffect(() => {
    if (id === null) return;
    let live = true;
    void commands.workoutGet(id).then((r) => {
      if (live && r.status === "ok") setDetail(r.data);
    });
    return () => {
      live = false;
    };
  }, [id]);
  return detail;
}

/** "next Cool-down 2:00 @ 120 W", shedding the word "next", then the watts, to fit (§16.1). */
function Next({
  block,
  last,
}: {
  block: WorkoutBlock | undefined;
  last: boolean;
}) {
  const name = block ? PHASE_NAME[block.phase] : "";
  const when = block?.durS == null ? name : `${name} ${clock(block.durS)}`;
  const w = block ? watts(block) : "";
  const [ref, text] = useFit<HTMLParagraphElement>(
    last
      ? ["ride complete → Review", "ride complete"]
      : block
        ? [`next ${when} @ ${w}`, `${when} @ ${w}`, when, name]
        : [""],
  );
  return (
    <p ref={ref} className="hero-next" data-tier="t3plus" data-slot="next">
      {text}
    </p>
  );
}

/** What the key row says of the trainer following its commanded mode (ADR-0002 §4.4.1). */
const FOLLOWING: Record<StreamFollowing, string | null> = {
  unknown: null,
  checking: "checking it follows",
  suspect: "may not be following",
  following: "following",
  not_following: "not following",
};

/** Records one subscription's samples into the session's history of `of`. */
function useRecord(
  session: RideSession,
  of: Series,
  s: { t: number | null; v: number | null } | null,
  workoutS: number | null,
) {
  const [t, v] = [s?.t ?? null, s?.v ?? null];
  useEffect(() => {
    if (t !== null && v !== null && workoutS !== null)
      session.record(t, workoutS, v, of);
  }, [session, of, t, v, workoutS]);
}

function Screen({ session }: { session: RideSession }) {
  const status = useStatus();
  const links = useLinks();
  // A wedged engine (#351): the stream's commands are refused after the core's bound, so the values below are stale.
  const unresponsive = useStreamError()?.code === "engine_unresponsive";
  const power = useSubscribe("power", { rateHz: 4 }).sample;
  const cadence = useSubscribe("cadence", { rateHz: 2 }).sample;
  const hr = useSubscribe("hr", { rateHz: 2 }).sample;
  const detail = useDetail(session);
  const compiled = detail?.compiled ?? null;
  const series = useSeries(session);
  const tooSmall = useTooSmall();

  const start = useCommand(commands.rideStart);
  const pause = useCommand(commands.ridePause);
  const resume = useCommand(commands.rideResume);
  const skip = useCommand(commands.rideSkip);
  const end = useCommand(commands.rideEnd);
  // The last refusal and the command it answered (the Ready sheet titles it by the command).
  const [refused, setRefused] = useState<{
    command: RideCommand;
    error: ApiError;
  } | null>(null);
  const refusal = refused?.error ?? null;
  const [confirming, setConfirming] = useState<"end" | "skip" | null>(null);
  const keepRiding = useRef<HTMLButtonElement>(null);
  const startButton = useRef<HTMLButtonElement>(null);
  // The safe answer holds focus while the dialog is open (WAI-ARIA alertdialog), as autoFocus did.
  useEffect(() => {
    if (confirming) keepRiding.current?.focus();
  }, [confirming]);
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
  // Each measured sample joins its history on the workout clock; a gap (`v` null) or a frozen clock adds nothing.
  useRecord(session, "power", power, workoutS);
  useRecord(session, "hr", hr, workoutS);
  useRecord(session, "cadence", cadence, workoutS);

  const run = async <T,>(
    command: RideCommand,
    go: () => Promise<
      { status: "ok"; data: T } | { status: "error"; error: ApiError }
    >,
  ) => {
    setRefused(null);
    const outcome = await go();
    if (outcome.status === "error")
      setRefused({ command, error: outcome.error });
    return outcome;
  };
  const startRide = async () => {
    setStartSent(true);
    const outcome = await run("start", start.run);
    if (outcome.status === "error") setStartSent(false);
  };
  const pauseOrResume = async (which: "pause" | "resume") => {
    setPauseSent(which);
    const outcome = await run(
      which,
      which === "pause" ? pause.run : resume.run,
    );
    if (outcome.status === "error") setPauseSent(null);
  };
  const skipBlock = async () => {
    setConfirming(null);
    setSkippedFrom(blockIdx);
    const outcome = await run("skip", skip.run);
    if (outcome.status === "error") setSkippedFrom(null);
  };
  const endRide = async () => {
    setConfirming(null);
    const outcome = await run("end", end.run);
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
  // Skip needs a next block to skip: none for a free ride (the core refuses, engine.rs `Engine::command` (~1266), the `SkipBlock` arm) and none known until the
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

  const next = blockIdx === null ? undefined : blocks[blockIdx + 1];
  // The plan dims while it does not run: the rider's pause and a trainer loss (§5.2 States).
  const dim = userPaused || loss.trainer ? "" : undefined;
  const xMax = Math.max(total ?? 0, workoutS ?? 0, 1);
  const free = session.workoutId === null;
  const counted = status?.compliance.countedS ?? 0;
  // The trainer as the engine commands it and whether it follows (the same words as the pause banner, #324).
  // Not while the trainer is lost: `commanded` then still holds the engine's last mode, which nothing is following.
  const trainer = loss.trainer ? null : trainerNow(status?.commanded);
  const follows =
    status && !loss.trainer ? FOLLOWING[status.trainerFollowing] : null;
  const following = status
    ? [
        trainer && follows
          ? `${trainer}, ${follows}`
          : (trainer ?? (follows && `trainer ${follows}`)),
        counted > 0
          ? `compliance ${Math.round((100 * (status.compliance.withinS ?? 0)) / counted)} %`
          : null,
      ]
        .filter(Boolean)
        .join(" · ")
    : "";

  // Before Start the traces column is the Ready sheet (ReadySheet.tsx, DESIGN §5.4): its state is the band's banner,
  // after an engine that does not answer (the values are not live); the sheet's plan line says a free ride is free.
  const ready = kind === "idle";
  const sheet = readyView({
    status: status !== null,
    links,
    hrLinked: session.loaded.hrLinked,
    refused: refused?.command ?? null,
  });
  // The usual choice is primary and focused (§5.4): focus goes to Start once it can be pressed, unless the rider has
  // put it on a control already.
  const startable =
    ready && status !== null && sheet.canStart && !busy && !startSent;
  const offeredStart = useRef(false);
  useEffect(() => {
    if (!startable || offeredStart.current) return;
    offeredStart.current = true;
    const at = document.activeElement;
    if (at === null || at === document.body || at.matches("[data-destination]"))
      startButton.current?.focus();
  }, [startable]);
  // The band shows every state that holds, most urgent first (banners.tsx `BANNER_ORDER`).
  const rideBanners = banners({
    unresponsive,
    loss,
    userPaused,
    free,
    commanded: status?.commanded,
    clock,
  });
  const bannerList = ready
    ? [
        ...rideBanners.filter((b) => b.id === "engine-unresponsive"),
        readyBanner(sheet),
      ]
    : rideBanners;
  const refusalLine = refusal && (
    <p role="alert" className="ride-refusal" data-code={refusal.code}>
      <strong>{REFUSAL[refusal.code] ?? "Failed"}:</strong> {refusal.message}
    </p>
  );
  const endHelp = (
    <button className="ride-end" onClick={() => setConfirming("end")}>
      End… <Key k="Esc" />
    </button>
  );

  return (
    <div className="ride" data-testid="ride">
      <header className="ride-bar" data-region="status">
        <span className="ride-title">
          {free ? "Free ride" : (detail?.workout.name ?? "")}
        </span>
        <span className="ride-times num" data-testid="times">
          <span data-slot="elapsed" style={{ width: "7ch" }}>
            {clock(workoutS)}
          </span>
          <span className="muted">
            {" "}
            / {total === null ? "—" : clock(total)} ·{" "}
          </span>
          <span data-slot="left" style={{ width: "7ch" }}>
            {total === null || workoutS === null
              ? "—"
              : clock(total - workoutS)}
          </span>
          <span className="muted"> left</span>
        </span>
        <span
          className="ride-chip num"
          data-slot="neo-back"
          data-testid="neo-back"
          style={{ width: "20ch" }}
        >
          {loss.backChip ? `NEO back · ${clock(loss.gapS)} gap` : ""}
        </span>
        <div className="ride-pill" role="group" aria-label="Devices">
          <Pill name="NEO" link={links.trainer} Icon={Bluetooth} />
          <Pill name="HR" link={links.hr} Icon={Heart} />
        </div>
        {detail?.workout.ftp != null && (
          <span className="chip num">FTP {detail.workout.ftp}</span>
        )}
        {tooSmall && (
          <p role="alert" className="too-small" data-tier="t3plus">
            {TOO_SMALL}
          </p>
        )}
      </header>

      <div className="ride-main">
        <section
          className="ride-traces panel"
          data-region="traces"
          aria-label="Traces"
        >
          <div className="banner-band" data-region="banner">
            <BannerBand list={bannerList} end={endHelp} />
          </div>
          {ready ? (
            <ReadyBody
              view={sheet}
              free={free}
              first={blocks[0]}
              clock={clock}
              refusal={refusalLine}
            />
          ) : (
            <div className="trace-stack">
              <Trace
                series="power"
                label="Power"
                unit="W"
                points={series.power}
                xMax={xMax}
                now={workoutS ?? 0}
                plan={blocks}
              />
              <Trace
                series="hr"
                label="Heart rate"
                unit="bpm"
                points={series.hr}
                xMax={xMax}
                now={workoutS ?? 0}
              />
              <Trace
                series="cadence"
                label="Cadence"
                unit="rpm"
                points={series.cadence}
                xMax={xMax}
                now={workoutS ?? 0}
              />
            </div>
          )}
        </section>

        <section
          className="ride-metrics panel"
          data-region="metrics"
          aria-label="Now"
        >
          <div className="readout hero" data-tile="countdown">
            <span className="readout-head">
              <Timer {...icon} />
              <span data-slot="block">
                {block
                  ? `${PHASE_WORD[block.phase]} · block ${(blockIdx ?? 0) + 1} of ${blocks.length}`
                  : status !== null && blocks.length > 0 && blockIdx === null
                    ? "Last block done"
                    : free
                      ? "FREE RIDE"
                      : "Loading the plan…"}
              </span>
            </span>
            {/* Under a trainer loss the countdown is frozen: the caption says where the block paused (§5.2). */}
            <span className="readout-sub" data-testid="caption">
              {loss.trainer
                ? `paused at ${clock(status?.blockElapsedS ?? null)}`
                : block?.durS == null
                  ? "in block"
                  : "left in block"}
            </span>
            <Num
              className="hero-t1"
              data-slot="countdown"
              data-tier="t1"
              data-live
              data-dim={dim}
            >
              {clock(blockLeft)
                .split(":")
                .flatMap((part, i) =>
                  i === 0
                    ? [part]
                    : [
                        <span key={i} className="colon">
                          :
                        </span>,
                        part,
                      ],
                )}
            </Num>
            <Next
              block={next}
              last={blocks.length > 0 && blockIdx === blocks.length - 1}
            />
          </div>
          <div className="readout" data-tile="power" data-series="power">
            <span className="readout-head">
              <Zap {...icon} />
              <span data-series-label>Power</span>
            </span>
            <span className="readout-sub" data-tile="target">
              TARGET <Value id="target" ch={8} value={targetView.value} />
              <span className="tile-unit">{targetView.unit}</span> ·{" "}
              <Value
                id="plan"
                ch={10}
                value={target === null ? "Plan —" : `Plan ${target} W`}
              />
            </span>
            <span className="readout-value">
              <Value
                id="power"
                ch={4}
                tier="t2"
                value={round(power?.v, loss.trainer)}
              />
              <span className="tile-unit">W</span>
            </span>
            {/* Reserved for the ERG band (§5.2): its marker and Δ hide while the trainer settles, which the status does not carry yet (#97). */}
            <span className="readout-band" aria-hidden />
          </div>
          <div className="readout" data-tile="hr" data-series="hr">
            <span className="readout-head">
              <Heart {...icon} />
              <span data-series-label>Heart rate</span>
            </span>
            <span className="readout-sub">
              {loss.hr ? "HR strap lost" : ""}
            </span>
            <span className="readout-value">
              <Value id="hr" ch={3} tier="t2" value={round(hr?.v, loss.hr)} />
              <span className="tile-unit">bpm</span>
            </span>
          </div>
          <div className="readout" data-tile="cadence" data-series="cadence">
            <span className="readout-head">
              <RotateCw {...icon} />
              <span data-series-label>Cadence</span>
            </span>
            <span className="readout-sub">
              {loss.erg ? "▲ spin faster" : ""}
            </span>
            <span className="readout-value">
              <Value
                id="cadence"
                ch={3}
                tier="t2"
                value={round(cadence?.v, loss.trainer)}
              />
              <span className="tile-unit">rpm</span>
            </span>
          </div>
        </section>
      </div>

      <footer className="ride-timeline panel" data-region="timeline">
        <Timeline blocks={blocks} totalS={total} workoutS={workoutS ?? 0} />
      </footer>

      <div className="ride-keys" data-region="keys">
        {/* Before the start the key row is the Ready sheet's (Start; the core refuses pause, resume and skip then);
            from the start it is the mockup's, with no Start (§18.3). */}
        {ready ? (
          <button
            ref={startButton}
            className="primary"
            disabled={busy || status === null || startSent || !sheet.canStart}
            onClick={() => void startRide()}
          >
            <Play {...icon} />
            Start <Key k="⏎" />
          </button>
        ) : (
          <>
            <button
              disabled={busy || !canPause}
              title={userPaused ? "Already paused" : undefined}
              onClick={() => void pauseOrResume("pause")}
            >
              <Pause {...icon} />
              Pause <Key k="Space" />
            </button>
            <button
              disabled={busy || !canResume}
              onClick={() => void pauseOrResume("resume")}
            >
              <Play {...icon} />
              Resume <Key k="Space" />
            </button>
            <button
              disabled={busy || !live || !canSkip || skipWaits}
              onClick={() =>
                skipEnds ? setConfirming("skip") : void skipBlock()
              }
            >
              <SkipForward {...icon} />
              Skip block <Key k="S" />
            </button>
          </>
        )}
        <div className="ride-status">
          {/* Before the start the line is the Ready sheet's: what Start waits for, never the sheet's words again; the
              trainer is not commanded yet, so nothing follows. */}
          <span data-testid="state">
            {ready ? sheet.line : stateLine(status)}
          </span>
          {!ready && following && <span className="muted"> · {following}</span>}
          {/* Before the start the refusal has its row on the Ready sheet. */}
          {!ready && refusalLine}
        </div>
        <button onClick={openKeys}>
          <Keyboard {...icon} />
          Keys <Key k="?" />
        </button>
        <button
          className="ride-end"
          disabled={finished || status === null}
          onClick={() => setConfirming("end")}
        >
          <Square {...icon} />
          End <Key k="Esc" />
        </button>
      </div>
      {confirming && (
        <div className="scrim">
          {/* eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions -- the WAI-ARIA dialog pattern closes on Escape from inside; the rule keys on element names, not roles, so it cannot allow a role=alertdialog. */}
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
            <button ref={keepRiding} onClick={() => setConfirming(null)}>
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
        <Screen session={session} />
      </StreamProvider>
    </div>
  );
}
