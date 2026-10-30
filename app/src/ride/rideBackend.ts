// The core's ride commands behind `mockIPC`, on top of the stream backend (`stream/testBackend.ts`), for the ride
// tests. It models every refusal the core returns, in the core's order, and accepts nothing the core refuses:
//
//   ride_start  no ride loaded            conflict            "no ride loaded"      crank-core core.rs `Core::engine` (~1666)
//               not in the loaded phase   conflict            "not valid in this state"  crank-engine engine.rs `Engine::command` (~1266), `Start` arm
//               engine has no trainer     device_unavailable  "no trainer"          engine.rs `command` `Start`
//               trainer link not ready    device_unavailable  "trainer not ready"   engine.rs `command` `Start`
//               health cap vs the blocks  conflict            "cap_requires_erg" / "cap_exceeded_target"  engine.rs `Engine::command` (~1266) `Start` arm, engine.rs `Session::check_cap` (~466)
//               record cannot be opened   storage             (the recorder's text) engine.rs `command` `Start`
//               (core.rs `Core::ensure_start_allowed` (~1236) always passes today and is not modelled)
//   ride_pause  no ride loaded            conflict            "no ride loaded"      core.rs `Core::engine`
//               not started / finished    conflict            "not valid in this state"  engine.rs `Engine::command` (~1266), the `Pause | Resume | SkipBlock` arm
//   ride_resume as pause, and not paused by the rider   conflict  "not valid in this state"  engine.rs `Engine::command` (~1266), `Resume` arm
//   ride_skip   as pause, and no block left (a free ride has none)  conflict  "not valid in this state"  engine.rs `Engine::command` (~1266), `SkipBlock` arm;
//               skipping the last block finishes the ride  engine.rs `command` `SkipBlock`
//   ride_end    no ride loaded or just finished  conflict     "no ride loaded"      core.rs `end_ride` (~1742)
//               ends a started ride (id), unloads an unstarted one (null), a finished one answers its id again  core.rs `end_ride` (reaped: `last`; `Stop` unless Finished, `Ended(finishes.then_some(id))`)
//               (`forced` again for a ride ended by force: `last_forced`, core.rs `end_ride`, set in core.rs `Core::force_end` (~1831))
//   ride_end    answers `{ rideId, forced }`: an engine that is gone or stuck is ended by force (`forced: true`, the ride's id,
//               recorded as aborted) instead of failing  crank-core core.rs `Core::end_ride` (~1742) / core.rs `Core::force_end` (~1831) (#331)
//   every other ride_* command, engine wedged  engine_unresponsive  (the core's text)  after the 1 s bound, core.rs
//               `bounded` / `REQUEST_TIMEOUT` via `ride_cmd` (#351); checked after "no ride loaded". The stream commands
//               answer the same (stream/testBackend.ts `truth.unresponsive`). `ride_end` ends such a ride by force.
//   every other ride_* command, engine task ended  internal  "engine stopped"  engine.rs `EngineHandle::send` (~226) via
//               core.rs `ride_cmd` (~1676), mapped error.rs `CommandError` (~65); checked after "no ride loaded" and before the engine judges the command.
//   workout_get unknown id                not_found           Library::get (library.rs)
// A call without its required argument is Tauri's plain-string rejection, not an `ApiError`.
import { mockIPC } from "@tauri-apps/api/mocks";
import type {
  ApiError,
  LinkChange,
  Phase,
  StreamCommanded,
  StreamPauseReason,
  StreamRideState,
  StreamStatus,
  WorkoutBlock,
  WorkoutDetail,
} from "../bindings";
import {
  UNRESPONSIVE,
  installBackend,
  status as makeStatus,
} from "../stream/testBackend";

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
  /** The ride the core ended by force; a repeat End answers it as forced again (core.rs `end_ride`, `last_forced`). */
  lastForced: string | null;
  rideId: string;
  userPause: boolean;
  /** The trainer link is lost in an active ride: the engine pauses with `trainer_lost`, which outranks a rider pause (engine.rs `Pauses::reason` (~336)). */
  trainerLost: boolean;
  /** 8 ticks without trainer samples on a link that is still ready: the engine pauses with `trainer_lost` all the same (engine.rs `PauseReason::TrainerLost` (~33)). */
  trainerSilent: boolean;
  /** The trainer is back and the rider is not pedalling yet: `state` stays `paused(trainer_lost)` until the first pedalling sample (#336). */
  pausedAfterLoss: boolean;
  /** Cadence and power were 0 for 3 s: `stopped_pedalling`, outranked by a rider pause and a trainer loss (engine.rs `Pauses::reason`). */
  stoppedPedalling: boolean;
  /** The strap link is lost: the ride keeps going (DESIGN §6.5). */
  hrLost: boolean;
  /**
   * ERG released for low cadence; the engine passes `policy.released` through (engine.rs `publish` (~1398)) and it
   * stays set until 60 rpm is held for 2 s (erg/policy.rs `ErgPolicy::update_release` (~118)), so it can accompany a pause.
   */
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
  /** The ERG watts the engine last sent while it is sending ERG (its soft start, steady target or ramp step); `null`: the current block's first planned watt, which is what a steady ERG block sends. */
  commandedW: number | null;
  /**
   * Resume has been answered and published, but the tick that sends the resumed mode has not run: `last_sent` is still
   * the pause's `Resistance { 0 }` (engine.rs `Resume` then `publish`, the policy runs on the next tick). `rig.tick()`
   * runs it. (The engine ticks at 4 Hz, ticks.rs `TICKS_PER_SECOND`; the mock's `tick()` is one *soft-start step*, i.e.
   * 1 s of the engine's 20 ticks, `SOFT_START_TICKS`. #357 tracks real engine↔mock parity.)
   */
  resumePending: boolean;
  /** A soft start is owed (a pause owes one, erg/policy.rs `leave_erg` (~148); Resume then ticks through it). */
  softOwed: boolean;
  /** The soft start's origin, the rider's 3 s power at its first pedalling tick; `null` until one (the engine holds `Erg { 0 }`). */
  rampOrigin: number | null;
  /** 1 Hz steps taken since the origin. */
  rampStep: number;
  /**
   * Samples stopped mid-ramp: the engine sends nothing and holds the last step (erg/policy.rs `ErgPolicy::desired` (~237),
   * `last_sent` is `Erg`); the ramp restarts from the then-current power when pedalling returns.
   */
  rampHold: boolean;
  blockIdx: number;
  trainer: "none" | "ready" | "connecting";
  cap: "none" | "requires_erg" | "exceeds";
  recordFails: boolean;
  /** The engine task has ended: every command that reaches it answers `internal`. */
  engineStopped: boolean;
  /** The engine is wedged: every command and stream request answers `engine_unresponsive` (#351). */
  engineUnresponsive: boolean;
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
    lastForced: null,
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
    commandedW: null,
    resumePending: false,
    softOwed: false,
    rampOrigin: null,
    rampStep: 0,
    rampHold: false,
    blockIdx: 0,
    trainer: "ready",
    cap: "none",
    recordFails: false,
    engineStopped: false,
    engineUnresponsive: false,
    publishes: true,
    blocks: BLOCKS,
    ...over,
  };
  // one engine: the stream commands are wedged with it
  base.truth.unresponsive = world.engineUnresponsive;
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
  /**
   * The engine's `commanded` (`publish`, engine.rs): its `last_sent`, only for the running ride. Rules, from the policy
   * (erg/policy.rs `ErgPolicy::desired` (~172)): a rider pause and a release both send `Resistance { 0 }` (the pause is judged first); a lost
   * link forgets the mode (`reset_control`) so nothing is commanded until it is re-issued; 8 silent ticks on a ready link
   * re-tell `Erg { 0 }`; a free block sends `Free`; otherwise ERG at the watts the engine is sending.
   */
  const commanded = (): StreamCommanded | null => {
    if (world.phase !== "active" || world.trainerLost) return null;
    if (world.userPause || world.resumePending)
      return { kind: "resistance", pct: 0 };
    // no block: a free ride, `Free` (erg/policy.rs `ErgPolicy::desired` (~196))
    const b = world.blocks[world.blockIdx];
    if (b === undefined) return { kind: "free" };
    // a free block is judged before the release (erg/policy.rs: no ERG block, no release): `Free`
    if (b.phase === "free") return { kind: "free" };
    if (world.ergReleased) return { kind: "resistance", pct: 0 };
    // link ready again but no pedalling sample yet (or 8 silent ticks): the engine holds `Erg { 0 }`
    if (world.trainerSilent || world.pausedAfterLoss)
      return { kind: "erg", watts: 0 };
    const target = world.commandedW ?? b.powerFrom ?? 0;
    if (world.softOwed) {
      // erg/policy.rs `ErgPolicy::desired` (~237): `Erg { 0 }` until a pedalling sample, then `origin + (target - origin) * step / 5`, step 0
      // being the origin itself; never the plan straight away.
      if (world.rampOrigin === null) return { kind: "erg", watts: 0 };
      const f = Math.min(world.rampStep, 5) / 5;
      return {
        kind: "erg",
        watts: Math.round(world.rampOrigin + (target - world.rampOrigin) * f),
      };
    }
    return { kind: "erg", watts: target };
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
      // the engine's `user_paused` (engine.rs publish): the rider's pause, also under a trainer loss
      userPaused: world.phase === "active" && world.userPause,
      commanded: commanded(),
    });
    base.truth.links = links();
    base.push({ status: base.truth.status, links: links() });
  };
  const reach = () => {
    if (world.phase === "none") throw err("conflict", "no ride loaded");
    if (world.engineUnresponsive)
      throw err("engine_unresponsive", UNRESPONSIVE);
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
        // the status goes out at once; the mode is re-sent by the next tick
        world.resumePending = true;
        world.softOwed = true;
        world.rampOrigin = null;
        world.rampStep = 0;
        world.rampHold = false;
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
          return { rideId: id, forced: id !== null && id === world.lastForced };
        }
        if (world.engineStopped || world.engineUnresponsive) {
          // Forced: the core ended the ride itself; it is recorded, as aborted.
          world.phase = "finished";
          world.lastForced = world.rideId;
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
    /** The engine wedges (#351): from now on the ride and stream commands answer `engine_unresponsive`. */
    wedge(): void {
      world.engineUnresponsive = true;
      base.truth.unresponsive = true;
    },
    /**
     * One soft-start step of the engine (the engine ticks at 4 Hz and steps the ramp once per second, 4 ticks; this
     * collapses those 4 ticks into one call, so call it once per second of ride; #357). `powerW` is the rider's 3 s power at a pedalling tick, `null` for no pedalling sample.
     */
    tick(powerW: number | null = null) {
      world.resumePending = false;
      if (world.softOwed) {
        if (powerW === null) {
          // no pedalling sample: before the ramp the engine holds `Erg { 0 }`; mid-ramp it sends nothing (holds the step)
          if (world.rampOrigin !== null) world.rampHold = true;
        } else if (world.rampOrigin === null || world.rampHold) {
          world.rampOrigin = powerW;
          world.rampStep = 0;
          world.rampHold = false;
        } else world.rampStep += 1;
        if (world.rampStep >= 5) world.softOwed = false;
      }
      publish();
    },
    /** Delay the answer of the next call of `cmd` (its effect on the core is at the call) until the returned function runs. */
    hold(cmd: string): () => void {
      let release = () => {};
      const gate = new Promise<void>((resolve) => (release = resolve));
      holds.set(cmd, [...(holds.get(cmd) ?? []), gate]);
      return release;
    },
  };
}
