// Ride (DESIGN §5.2, §6.1, §6.4, §16.1; #96 MVP subset, #256): ride the loaded workout in ERG with live tiles, the
// whole-ride overview and pause / skip / end. It renders what the stream and the core's answers say and branches
// on `ApiError.code`, never on `message` (ADR-0001 §4.4); the core's text is shown as the reason of a refusal.
//
// Out of this subset, each with its issue: density modes, the coach lane, the road (#96), sound and the input
// dispatcher, the failure overlays (#100: here a fault is the device pill and the state line), the Review/summary
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
import { StreamProvider, useCommand, useLinks, useStatus } from "../stream";
import { useSubscribe } from "../stream";
import { RideTiers } from "../useTypeTiers";
import { RideChart } from "./RideChart";
import { usePower, useSession, type RideSession } from "./session";

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
        : s.reason === "trainer_lost"
          ? "Paused: trainer lost, reconnecting"
          : "Paused: waiting for you to pedal";
    case "riding":
      return st.ergReleased ? "ERG released: spin up to re-engage" : "Riding";
  }
}

const REFUSAL: Record<string, string> = {
  conflict: "Not possible right now",
  device_unavailable: "Trainer not ready",
  storage: "The ride could not be recorded",
};

/** One value of the grid: its slot reserves the widest form (`ch`), so a longer number moves nothing (P4, §16.1). */
function Tile({
  id,
  label,
  unit,
  ch,
  value,
}: {
  id: string;
  label: string;
  unit: string;
  ch: number;
  value: number | string | null;
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
  const [confirming, setConfirming] = useState(false);

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

  const run = async (
    go: () => Promise<
      { status: "ok"; data: unknown } | { status: "error"; error: ApiError }
    >,
  ) => {
    setRefusal(null);
    const outcome = await go();
    if (outcome.status === "error") setRefusal(outcome.error);
    return outcome;
  };
  const endRide = async () => {
    setConfirming(false);
    const outcome = await run(end.run);
    if (outcome.status === "ok")
      session.leaveOnce(() =>
        typeof outcome.data === "string"
          ? navigate("history", outcome.data)
          : navigate("workouts"),
      );
  };

  const blocks = compiled?.blocks ?? [];
  const blockIdx = status?.blockIndex ?? null;
  const block = blockIdx === null ? undefined : blocks[blockIdx];
  const target = targetNow(block, status?.blockElapsedS ?? null);
  const blockLeft =
    block?.durS != null && status?.blockElapsedS != null
      ? block.durS - status.blockElapsedS
      : (status?.blockElapsedS ?? null);
  const total = compiled?.totalS ?? null;
  const userPaused =
    status?.state.kind === "paused" && status.state.reason === "user";
  const busy =
    start.pending ||
    pause.pending ||
    resume.pending ||
    skip.pending ||
    end.pending;
  const live = kind === "riding" || kind === "paused";
  const round = (v: number | null | undefined) =>
    v == null ? null : Math.round(v);

  return (
    <div className="ride" data-testid="ride">
      <header className="ride-bar">
        <div className="ride-controls">
          {kind === "idle" ? (
            <button
              className="primary ride-btn"
              disabled={busy || status === null}
              onClick={() => void run(start.run)}
            >
              Start
            </button>
          ) : (
            <button
              className="ride-btn"
              disabled={busy || !live}
              onClick={() => void run(userPaused ? resume.run : pause.run)}
            >
              {userPaused ? "Resume" : "Pause"}
            </button>
          )}
          <button
            className="ride-btn"
            disabled={busy || !live}
            onClick={() => void run(skip.run)}
          >
            Skip block
          </button>
          <button
            className="ride-btn"
            disabled={busy || finished || status === null}
            onClick={() => setConfirming(true)}
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
          <Tile id="target" label="TARGET" unit="W" ch={4} value={target} />
          <Tile
            id="power"
            label="POWER"
            unit="W"
            ch={4}
            value={round(power?.v)}
          />
        </div>
      </section>

      <aside className="ride-side" aria-label="Body">
        <Tile
          id="hr"
          label="HEART RATE"
          unit="bpm"
          ch={3}
          value={round(hr?.v)}
        />
        <Tile
          id="cadence"
          label="CADENCE"
          unit="rpm"
          ch={3}
          value={round(cadence?.v)}
        />
        <div className="coach-cell" data-cell="coach" aria-hidden />
      </aside>

      <div className="ride-notice">
        <p className="ride-state" data-testid="state">
          {stateLine(status)}
        </p>
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
          >
            <h2 id="end-title">End this ride?</h2>
            <p>
              {kind === "idle"
                ? "The ride has not started; it will be unloaded."
                : "The ride is saved up to now and cannot be resumed."}
            </p>
            <button autoFocus onClick={() => setConfirming(false)}>
              Keep riding
            </button>
            <button className="primary" onClick={() => void endRide()}>
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
