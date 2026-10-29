// The core's ride commands behind `mockIPC`, on top of the stream backend (`stream/testBackend.ts`), for the ride
// tests. It models every refusal the core returns, in the core's order, and accepts nothing the core refuses:
//
//   ride_start  no ride loaded            conflict            "no ride loaded"      crank-core core.rs:968 (`engine`)
//               not in the loaded phase   conflict            "not valid in this state"  crank-engine engine.rs:1180
//               engine has no trainer     device_unavailable  "no trainer"          engine.rs:1183
//               trainer link not ready    device_unavailable  "trainer not ready"   engine.rs:1186
//               health cap vs the blocks  conflict            "cap_requires_erg" / "cap_exceeded_target"  engine.rs:1187, 379-392
//               record cannot be opened   storage             (the recorder's text) engine.rs:1188
//               (`ensure_start_allowed`, core.rs:619, always passes today and is not modelled)
//   ride_pause  no ride loaded            conflict            "no ride loaded"      core.rs:968
//               not started / finished    conflict            "not valid in this state"  engine.rs:1191-1198
//   ride_resume as pause, and not paused by the rider   conflict  "not valid in this state"  engine.rs:1207
//   ride_skip   as pause, and no block left (a free ride has none)  conflict  "not valid in this state"  engine.rs:1212;
//               skipping the last block finishes the ride  engine.rs:1217-1219
//   ride_end    no ride loaded or just finished  conflict     "no ride loaded"      core.rs:1028
//               ends a started ride (id), unloads an unstarted one (null), a finished one answers its id again  core.rs:1037-1049
//   ride_end    answers `{ rideId, forced }`: an engine that is gone or stuck is ended by force (`forced: true`, the ride's id,
//               recorded as aborted) instead of failing  crank-core core.rs `end_ride` / `force_end` (#331)
//   every other ride_* command, engine task ended  internal  "engine stopped"  `EngineHandle::send` engine.rs:156,193-194 via
//               core.rs:971, mapped error.rs:69; checked after "no ride loaded" and before the engine judges the command.
//   workout_get unknown id                not_found           Library::get (library.rs)
// A call without its required argument is Tauri's plain-string rejection, not an `ApiError`.
import { mockIPC } from "@tauri-apps/api/mocks";
import type {
  ApiError,
  LinkChange,
  Phase,
  StreamPauseReason,
  StreamRideState,
  StreamStatus,
  WorkoutBlock,
  WorkoutDetail,
} from "../bindings";
import { installBackend, status as makeStatus } from "../stream/testBackend";

export const block = (
  startS: number,
  durS: number,
  from: number,
  to: number = from,
  phase: Phase = "work",
): WorkoutBlock => ({
  path: [0],
  iteration: [],
  nodeId: `n${startS}`,
  startS,
  durS,
  powerFrom: from,
  powerTo: to,
  phase,
  durRangeS: null,
});

/** Three blocks, 10 minutes: warm-up ramp, steady work, cool-down. */
export const BLOCKS: WorkoutBlock[] = [
  block(0, 180, 100, 180, "warmup"),
  block(180, 300, 250, 250, "work"),
  block(480, 120, 120, 100, "cooldown"),
];

const err = (code: ApiError["code"], message: string): ApiError => ({
  code,
  message,
});

export interface RideWorld {
  /** A ride is loaded; the phase of it. */
  phase: "none" | "loaded" | "active" | "finished";
  /** Reaped ride whose end is still answerable: `ride_end` returns its id. */
  lastId: string | null;
  rideId: string;
  userPause: boolean;
  /** The trainer link is lost in an active ride: the engine pauses with `trainer_lost`, which outranks a rider pause (engine.rs:295). */
  trainerLost: boolean;
  /** 8 ticks without trainer samples on a link that is still ready: the engine pauses with `trainer_lost` all the same (engine.rs:~33). */
  trainerSilent: boolean;
  /** The trainer is back and the rider is not pedalling yet: `state` stays `paused(trainer_lost)` until the first pedalling sample (#336). */
  pausedAfterLoss: boolean;
  /** Cadence and power were 0 for 3 s: `stopped_pedalling`, outranked by a rider pause and a trainer loss (engine.rs:295-306). */
  stoppedPedalling: boolean;
  /** The strap link is lost: the ride keeps going (DESIGN §6.5). */
  hrLost: boolean;
  /** ERG released for low cadence; the engine passes `policy.released` through (engine.rs:1279) and it stays set until 60 rpm is held for 2 s (erg/policy.rs:116-124), so it can accompany a pause. */
  ergReleased: boolean;
  /** The session clock the published status carries (the heartbeat refreshes it in the core). */
  sessionS: number;
  /** Session second the current loss began; defaults to `sessionS` when a loss is published without one. */
  lostSinceS: number | null;
  /** The driver's reconnect attempt in the current loss; reported only while lost (engine `RideStatus`, #329). */
  attempt: number | null;
  /** The soft start's target after a recovered loss; reported only while not lost. */
  softStartW: number | null;
  /** Length of the last loss and when it ended; reported only while not lost (the next loss clears them). */
  gapS: number | null;
  reconnectedAtS: number | null;
  blockIdx: number;
  trainer: "none" | "ready" | "connecting";
  cap: "none" | "requires_erg" | "exceeds";
  recordFails: boolean;
  /** The engine task has ended: every command that reaches it answers `internal`. */
  engineStopped: boolean;
  /** Publish the status frame after a command (the core does; false models the frame still on its way). */
  publishes: boolean;
  blocks: WorkoutBlock[];
}

export function installRide(over: Partial<RideWorld> = {}) {
  const base = installBackend();
  const inner = (
    window as unknown as {
      __TAURI_INTERNALS__: {
        invoke: (cmd: string, args?: unknown) => Promise<unknown>;
      };
    }
  ).__TAURI_INTERNALS__.invoke;
  const world: RideWorld = {
    phase: "loaded",
    lastId: null,
    rideId: "ride-1",
    userPause: false,
    trainerLost: false,
    trainerSilent: false,
    pausedAfterLoss: false,
    stoppedPedalling: false,
    hrLost: false,
    ergReleased: false,
    sessionS: 0,
    lostSinceS: null,
    attempt: null,
    softStartW: null,
    gapS: null,
    reconnectedAtS: null,
    blockIdx: 0,
    trainer: "ready",
    cap: "none",
    recordFails: false,
    engineStopped: false,
    publishes: true,
    blocks: BLOCKS,
    ...over,
  };
  const calls: { cmd: string; args: Record<string, unknown> }[] = [];
  const holds = new Map<string, Promise<void>[]>();

  const state = (): StreamRideState => {
    if (world.phase === "finished") return { kind: "finished" };
    if (world.phase !== "active") return { kind: "idle" };
    if (world.trainerLost || world.trainerSilent || world.pausedAfterLoss)
      return {
        kind: "paused",
        reason: "trainer_lost" satisfies StreamPauseReason,
      };
    if (world.userPause)
      return { kind: "paused", reason: "user" satisfies StreamPauseReason };
    return world.stoppedPedalling
      ? {
          kind: "paused",
          reason: "stopped_pedalling" satisfies StreamPauseReason,
        }
      : { kind: "riding" };
  };
  const links = (): LinkChange[] =>
    world.phase === "active"
      ? [
          {
            device: "trainer",
            state: world.trainerLost ? "lost" : "ready",
          },
          { device: "hr", state: world.hrLost ? "lost" : "ready" },
        ]
      : [];
  /**
   * The loss fields as the engine reports them: only while the session is Active, `trainerLostSinceS` and the attempt
   * only while lost, the soft start, the last gap and its end only while not (a new loss clears them), and `userPaused`
   * apart from `state` (crank-engine `publish`, #329/#336).
   */
  const lossFields = (): Partial<StreamStatus> => {
    if (world.phase !== "active") return {};
    const lost = world.trainerLost || world.trainerSilent;
    return {
      userPaused: world.userPause,
      trainerLostSinceS: lost ? (world.lostSinceS ?? world.sessionS) : null,
      reconnectAttempt: lost ? world.attempt : null,
      softStartTargetW: lost ? null : world.softStartW,
      lastGapS: lost ? null : world.gapS,
      reconnectedAtS: lost ? null : world.reconnectedAtS,
    };
  };
  /** The engine's `publish`: the status frame after a command. */
  let clockS = 0;
  const publish = (workoutS?: number) => {
    // The session clock only runs forward (engine `session_s`): a test that rewinds it models a status the core cannot send.
    if (world.sessionS < clockS)
      throw new Error(
        `session clock went backwards: ${clockS} -> ${world.sessionS}`,
      );
    clockS = world.sessionS;
    if (!world.publishes) return;
    base.truth.status = makeStatus({
      ...lossFields(),
      sessionS: world.sessionS,
      state: state(),
      loaded: world.phase !== "none",
      workoutS: workoutS ?? base.truth.status.workoutS,
      blockIndex: world.blockIdx < world.blocks.length ? world.blockIdx : null,
      blockElapsedS: 0,
      ergReleased: world.ergReleased,
    });
    base.truth.links = links();
    base.push({ status: base.truth.status, links: links() });
  };
  const reach = () => {
    if (world.phase === "none") throw err("conflict", "no ride loaded");
    if (world.engineStopped) throw err("internal", "engine stopped");
  };
  const active = () => {
    reach();
    if (world.phase !== "active")
      throw err("conflict", "not valid in this state");
  };

  const handle = (cmd: string, args: Record<string, unknown>): unknown => {
    switch (cmd) {
      case "ride_start":
        reach();
        if (world.phase !== "loaded")
          throw err("conflict", "not valid in this state");
        if (world.trainer === "none")
          throw err("device_unavailable", "no trainer");
        if (world.trainer !== "ready")
          throw err("device_unavailable", "trainer not ready");
        if (world.cap === "requires_erg")
          throw err("conflict", "cap_requires_erg");
        if (world.cap === "exceeds")
          throw err("conflict", "cap_exceeded_target");
        if (world.recordFails) throw err("storage", "cannot open the record");
        world.phase = "active";
        publish();
        return null;
      case "ride_pause":
        active();
        world.userPause = true;
        publish();
        return null;
      case "ride_resume":
        active();
        if (!world.userPause) throw err("conflict", "not valid in this state");
        world.userPause = false;
        publish();
        return null;
      case "ride_skip":
        active();
        if (world.blockIdx >= world.blocks.length)
          throw err("conflict", "not valid in this state");
        world.blockIdx += 1;
        if (world.blockIdx >= world.blocks.length) world.phase = "finished";
        publish();
        return null;
      case "ride_end": {
        if (world.phase === "none" || world.phase === "finished") {
          const id = world.phase === "finished" ? world.rideId : world.lastId;
          if (id === null) throw err("conflict", "no ride loaded");
          return { rideId: id, forced: false };
        }
        if (world.engineStopped) {
          // Forced: the core ended the ride itself; it is recorded, as aborted.
          world.phase = "finished";
          publish();
          return { rideId: world.rideId, forced: true };
        }
        const started = world.phase === "active";
        if (started) {
          world.phase = "finished";
          publish();
          return { rideId: world.rideId, forced: false };
        }
        world.phase = "none";
        publish();
        return { rideId: null, forced: false };
      }
      case "workout_get": {
        if (typeof args.id !== "string")
          throw "missing required key id for command `workout_get`";
        if (args.id !== "w1") throw err("not_found", "unknown workout");
        return {
          workout: {},
          dsl: "",
          compiled: {
            blocks: world.blocks,
            totalS: 600,
            totalRangeS: [600, 600],
            tss: 10,
            if: 0.7,
            timeInZoneS: [0, 0, 0, 0, 0, 0, 0],
            issues: [],
          },
        } as unknown as WorkoutDetail;
      }
      default:
        return undefined;
    }
  };

  const ours = new Set([
    "ride_start",
    "ride_pause",
    "ride_resume",
    "ride_skip",
    "ride_end",
    "workout_get",
  ]);
  mockIPC((cmd, payload) => {
    const args = (payload ?? {}) as Record<string, unknown>;
    if (!ours.has(cmd)) return inner(cmd, payload);
    calls.push({ cmd, args });
    let result: unknown;
    let failure: { error: unknown } | null = null;
    try {
      result = handle(cmd, args);
    } catch (error) {
      failure = { error };
    }
    const gate = holds.get(cmd)?.shift();
    const settle = () => {
      if (failure) throw failure.error;
      return result;
    };
    return gate ? gate.then(settle) : settle();
  });
  base.truth.status = makeStatus({
    state: state(),
    loaded: world.phase !== "none",
    blockIndex: 0,
  });
  return {
    base,
    world,
    calls,
    count: (cmd: string) => calls.filter((c) => c.cmd === cmd).length,
    publish,
    /** Delay the answer of the next call of `cmd` (its effect on the core is at the call) until the returned function runs. */
    hold(cmd: string): () => void {
      let release = () => {};
      const gate = new Promise<void>((resolve) => (release = resolve));
      holds.set(cmd, [...(holds.get(cmd) ?? []), gate]);
      return release;
    },
  };
}
