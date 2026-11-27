// The core's ride commands behind `mockIPC`, on top of the snapshot channel's backend (`stream/uiBackend.ts`, which
// models `ui_open` and `ui_ack`), for the ride tests and the design fixtures. Every effect reaches the page as the
// channel's next frame (ADR-0001 §4.3): `ride` from the engine's status, and `devices` with the ride's link overlaid on
// each device it holds (§4.3.6). It models every refusal the core returns, in the core's order, and accepts nothing
// the core refuses:
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
//               answers `null` (app/src-tauri commands.rs `ride_end`): it ends a started ride (`ride.ended: finished`),
//               unloads an unstarted one (`ride: null`, core.rs `Core::end_ride`, ui.rs `Ui::unload`), and answers a
//               finished one again  core.rs `end_ride` (reaped: `last`; `Stop` unless Finished)
//               an engine that is gone or stuck is ended by force instead of failing (`ride.ended: forced`, recorded as
//               aborted)  crank-core core.rs `Core::end_ride` (~1742) / core.rs `Core::force_end` (~1831) (#331), ui.rs `Ui::ended`
//   every other ride_* command, engine wedged  engine_unresponsive  (the core's text)  after the 1 s bound, core.rs
//               `bounded` / `REQUEST_TIMEOUT` via `ride_cmd` (#351); checked after "no ride loaded". The frame's
//               `ride.engineUnresponsive` says so (core.rs `Core::watched`, ui.rs `Ui::unresponsive`). `ride_end` ends such a
//               ride by force.
//   every other ride_* command, engine task ended  internal  "engine stopped"  engine.rs `EngineHandle::send` (~226) via
//               core.rs `ride_cmd` (~1676), mapped error.rs `ApiError` (~65), its `From<CommandError>`; checked after "no ride loaded" and before the engine judges the command.
//   workout_get unknown id                not_found           Library::get (library.rs)
//   ride_plan   not the ride installed last  not_found     core.rs `Core::ride_plan` (the loaded ride's own workout, #110)
//   links_intent (the link keeper, ADR-0002 §5.1; answers `null` once the intent is applied, #597: its effect reaches the
//               page only as the snapshot channel's next frame, in `devices`), in the core's order:
//               "Release trainer" while the ride's session is started: an active ride, or one that finished (its last
//                 block skipped) and that the reaper has not taken yet (`world.unreaped`)  conflict
//                 "the ride holds the trainer: end the ride first"  core.rs `Core::links_intent` (~1250), the
//                 session's state is not `Idle`; after `ride_end` (which waits for the reap,
//                 core.rs `Core::end_ride` (~2225)) or the reap, the ride is gone and it is accepted
//                 (keeper/mod.rs `Keeper::free_intent` (~541))
//               the keeper's pairings cannot be read when it first starts  storage  core.rs `Core::keeper` (~1217),
//                 whose core.rs `Core::pairings` (~1207) reads the store
//               a release recorded while the links were handed to a load in flight (no ride loaded yet), superseded by a
//                 later rider action ("Ride this", Retry; #520)  conflict  keeper/run.rs `Task::answer` (~604), text
//                 keeper/mod.rs `SUPERSEDED` (~316); recorded only in `Ride::Handed`
//                 (keeper/mod.rs `Keeper::intent` (~458); `world.supersedes`)
//               `likely` / `retry` after the quit  conflict  "the application is quitting"
//                 keeper/mod.rs `Keeper::free_intent` (~541), text keeper/mod.rs `QUITTING` (~314)
//               a "Release trainer" whose discard raced a Start and found the ride started  conflict
//                 "the ride was started or is gone: it was not discarded"  keeper/run.rs `Task::internal` (~492); not
//                 reachable here (the mock's commands are atomic, so a Start never races the discard)
//               ("the link keeper is gone", `internal`, keeper/run.rs `Handle::intent` (~206), needs a dropped core: not
//                 modelled)
//               `sleep` or `rider_release` with a loaded, unstarted ride discards it (the path of Cancel); every other
//               intent leaves a loaded or started ride alone (keeper/mod.rs `Keeper::loaded_intent` (~490))
// A call without its required argument is Tauri's plain-string rejection, not an `ApiError`.
import { mockIPC } from "@tauri-apps/api/mocks";
import type {
  ApiError,
  AthleteSnapshotView,
  DeviceLinkView,
  DeviceView,
  LinkIntent,
  LinkStatus,
  LinkView,
  LiveValues,
  PairedDevice,
  Phase,
  RideEnding,
  StreamCommanded,
  StreamPauseReason,
  StreamRideState,
  StreamStatus,
  WorkoutBlock,
  RidePlan,
  WorkoutDetail,
} from "../bindings";
import { UNRESPONSIVE, status as makeStatus } from "../stream/testBackend";
import { SUPERSEDED, installUi, type UiOptions } from "../stream/uiBackend";

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
/** The keeper's answer to a superseded release (stream/uiBackend.ts models the keeper without a ride). */
export { SUPERSEDED };
const cold = (d: DeviceLinkView | null): DeviceLinkView | null =>
  d && { phase: "cold", sample: null };
/** The keeper's state with both devices paired and `cold`, as at launch. */
export const COLD_LINKS: LinkView = {
  trainer: { phase: "cold", sample: null },
  strap: { phase: "cold", sample: null },
};

/** The paired trainer and strap of the mock's core. */
export const NEO: PairedDevice = {
  role: "trainer",
  id: "neo-1",
  name: "Tacx Neo 55624",
  serial: "55624",
  model: "Neo",
};
export const HRM: PairedDevice = {
  role: "heart_rate",
  id: "hrm-1",
  name: "HRM-Dual",
  serial: null,
  model: null,
};
/** A fresh store's athlete model (crank-domain `athlete::prior`): FTP and CP 250 W ± 25 %, W′ 20 kJ ± 30 %, the rest unset. */
export const PRIOR_SNAPSHOT: AthleteSnapshotView = {
  ftp: { value: 250, sd: 62.5, source: "prior", estimateId: null },
  cp: { value: 250, sd: 62.5, source: "prior", estimateId: null },
  wPrime: { value: 20000, sd: 6000, source: "prior", estimateId: null },
  pmax: null,
  hrMax: null,
  lthr: null,
  hrRest: null,
  weight: null,
};
const NO_LIVE: LiveValues = { powerW: null, cadenceRpm: null, hrBpm: null };

export interface RideWorld {
  /** A ride is loaded; the phase of it. */
  phase: "none" | "loaded" | "active" | "finished";
  /** How the loaded ride ended, kept until the next load (ui.rs `Ui::ended`); `null` while it runs. */
  ended: RideEnding | null;
  /** The loaded ride's workout, `null` for a free ride (`ride.workoutId`). */
  workoutId: string | null;
  /** This tick's values, the frame's `ride.live` (the engine nulls them while paused or lost, ui.rs `Ui::read`). */
  live: LiveValues;
  /** What is paired per role (`devices.<role>.paired`). */
  paired: { trainer: PairedDevice | null; strap: PairedDevice | null };
  /** Reaped ride whose end is still answerable: `ride_end` returns its id. */
  lastId: string | null;
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
   * stays set until the re-engage cadence is held for its dwell (`TUNING_DEFAULT.erg`;
   * erg/policy.rs `ErgPolicy::update_release` (~138)), so it can accompany a pause.
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
  /** The trainer link before and between losses: `ride_start` needs it `ready`; `lost` is a link that went down. */
  trainer: "none" | "ready" | "connecting" | "lost";
  cap: "none" | "requires_erg" | "exceeds";
  recordFails: boolean;
  /** The engine task has ended: every command that reaches it answers `internal`. */
  engineStopped: boolean;
  /** The engine is wedged: every ride command answers `engine_unresponsive` and the frame says so (#351). */
  engineUnresponsive: boolean;
  /** Publish the status frame after a command (the core does; false models the frame still on its way). */
  publishes: boolean;
  /**
   * The ride finished on its own (its last block skipped) and the reaper has not taken its session yet: it still holds
   * its devices and refuses "Release trainer" (core.rs `Core::links_intent`). `ride_end` waits for the reap and
   * `reap()` runs it (core.rs `Core::reap` (~1626)); a ride ended by `ride_end` is already reaped.
   */
  unreaped: boolean;
  blocks: WorkoutBlock[];
  /** The workout's name as `workout_get` answers it. */
  workoutName: string;
  /**
   * The athlete model frozen at the load (`RidePlan.athlete`, ADR-0009 §2.1.4; core.rs `Core::ride_load`); a fresh
   * store's by default. A later change to the model (Settings) never reaches it.
   */
  athlete: AthleteSnapshotView;
  /** The link keeper's state of the devices no ride holds (`devices.<role>.phase` and `.sample`). */
  keeper: LinkView;
  /**
   * The strap was warm at the load and the ride holds it (`devices.strap.phase` is `riding`). When false it stayed the
   * keeper's and keeps its own phase, e.g. `not_found` for a strap that did not advertise (keeper/mod.rs
   * `Keeper::settle` hands over only `warm` devices; core.rs `Core::connect_links`).
   */
  strapHanded: boolean;
  /** The keeper's store read fails when it first starts: `links_intent` answers `storage`. */
  keeperStorage: boolean;
  /** The next release reaches the keeper while the links are handed to a load, and a rider action supersedes it. */
  supersedes: boolean;
  /** The app is quitting: `likely` and `retry` are refused. */
  quitting: boolean;
}

export function installRide(over: Partial<RideWorld> = {}, ui: UiOptions = {}) {
  const channel = installUi(ui);
  const inner = (
    window as unknown as {
      __TAURI_INTERNALS__: {
        invoke: (cmd: string, args?: unknown) => Promise<unknown>;
      };
    }
  ).__TAURI_INTERNALS__.invoke;
  const world: RideWorld = {
    phase: "loaded",
    ended: null,
    workoutId: "w1",
    live: NO_LIVE,
    paired: { trainer: NEO, strap: HRM },
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
    unreaped: false,
    blocks: BLOCKS,
    workoutName: "Steady 10 min",
    athlete: PRIOR_SNAPSHOT,
    keeper: COLD_LINKS,
    strapHanded: true,
    keeperStorage: false,
    supersedes: false,
    quitting: false,
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
  /**
   * The ride's link of each device it holds (engine `RideStatus`, which carries it beside the status, so a loss and
   * its pause share a frame, ADR-0001 §4.3.6): the trainer's from `trainer` and the loss, the strap's from `hrLost`.
   */
  const rideLink = {
    trainer: (): LinkStatus =>
      world.trainerLost || world.trainer === "lost" || world.trainer === "none"
        ? "lost"
        : world.trainer === "connecting"
          ? "connecting"
          : "ready",
    strap: (): LinkStatus => (world.hrLost ? "lost" : "ready"),
  };
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
  /** The engine's last published status. */
  let current: StreamStatus = makeStatus({
    state: state(),
    loaded: world.phase !== "none",
    blockIndex: 0,
  });
  /** The engine's `publish`: the status after a command, and the frame it causes. */
  let clockS = 0;
  const publish = (workoutS?: number) => {
    // The session clock only runs forward (engine `session_s`): a test that rewinds it models a status the core cannot send.
    if (world.sessionS < clockS)
      throw new Error(
        `session clock went backwards: ${clockS} -> ${world.sessionS}`,
      );
    clockS = world.sessionS;
    settleLinks();
    // The status is still on its way; a device the ride let go is the keeper's at once (the next frame carries it).
    if (!world.publishes) {
      show(false);
      return;
    }
    current = makeStatus({
      ...lossFields(),
      sessionS: world.sessionS,
      state: state(),
      loaded: world.phase !== "none",
      workoutS: workoutS ?? current.workoutS,
      blockIndex: world.blockIdx < world.blocks.length ? world.blockIdx : null,
      blockElapsedS: 0,
      ergReleased: world.ergReleased,
      // the engine's `user_paused` (engine.rs publish): the rider's pause, also under a trainer loss
      userPaused: world.phase === "active" && world.userPause,
      commanded: commanded(),
    });
    show();
  };
  /** The ride's session is started: an active ride, or a finished one the reaper has not taken yet. */
  const started = () =>
    world.phase === "active" || (world.phase === "finished" && world.unreaped);
  /**
   * A loaded or started ride holds the devices it was handed: the trainer, and the strap when `strapHanded`. A
   * reaped ride (ended, or finished and taken by the reaper) and an unloaded one hold nothing: the hold is dropped
   * with the session (core.rs `Core::reap`).
   */
  const rideHolds = () => world.phase === "loaded" || started();
  let held = rideHolds();
  /**
   * `devices` (ADR-0001 §4.3.6, ui.rs `Ui::read`): per role its pairing; a device the ride holds is `riding` with the
   * ride's link, every other one the keeper's phase and sample; `phase` is `null` exactly when nothing is paired.
   */
  const device = (
    paired: PairedDevice | null,
    keeper: DeviceLinkView | null,
    holds: boolean,
    link: () => LinkStatus,
  ): DeviceView => {
    if (paired === null)
      return { paired: null, phase: null, link: null, sample: null };
    if (holds) return { paired, phase: "riding", link: link(), sample: null };
    return {
      paired,
      phase: keeper?.phase ?? "cold",
      link: null,
      sample: keeper?.sample ?? null,
    };
  };
  /**
   * The frame's state from the world, sent as the core's next frame; without `ride` only the devices move (the
   * engine's status, and the end it carries, are still on their way).
   */
  const show = (ride = true) => {
    const holds = rideHolds();
    channel.truth.devices = {
      trainer: device(
        world.paired.trainer,
        world.keeper.trainer,
        holds,
        rideLink.trainer,
      ),
      strap: device(
        world.paired.strap,
        world.keeper.strap,
        holds && world.strapHanded,
        rideLink.strap,
      ),
    };
    if (!ride) {
      channel.changed();
      return;
    }
    channel.truth.ride =
      world.phase === "none"
        ? null
        : {
            id: world.rideId,
            workoutId: world.workoutId,
            status: current,
            live: world.live,
            ended: world.ended,
            // set by a timed-out request or a silent status, kept until the ride ends (ui.rs `Ui::read`)
            engineUnresponsive:
              world.engineUnresponsive && world.ended === null,
          };
    channel.changed();
  };
  /**
   * The ride's hold is released (an unload, a discard, a ride's end): the devices it held come back `cold`, with no
   * sample, and a strap it never held keeps its phase (keeper/mod.rs `Keeper::free`, `Phase::Cold`).
   */
  const settleLinks = () => {
    const now = rideHolds();
    if (held && !now)
      world.keeper = {
        trainer: cold(world.keeper.trainer),
        strap: world.strapHanded
          ? cold(world.keeper.strap)
          : world.keeper.strap,
      };
    held = now;
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
        if (world.blockIdx >= world.blocks.length) {
          world.phase = "finished";
          world.unreaped = true;
        }
        publish();
        return null;
      case "ride_end": {
        if (world.phase === "none" || world.phase === "finished") {
          const id = world.phase === "finished" ? world.rideId : world.lastId;
          if (id === null) throw err("conflict", "no ride loaded");
          // an End of a ride that finished on its own waits for its reap (core.rs `Core::settle`)
          world.unreaped = false;
          settleLinks();
          show();
          return null;
        }
        if (world.engineStopped || world.engineUnresponsive) {
          // Forced: the core ended the ride itself; it is recorded, as aborted.
          world.phase = "finished";
          world.ended = "forced";
          publish();
          return null;
        }
        if (world.phase === "active") {
          world.phase = "finished";
          world.ended = "finished";
          publish();
          return null;
        }
        world.phase = "none";
        publish();
        return null;
      }
      case "links_intent": {
        const intent = args.intent as LinkIntent | undefined;
        if (intent === undefined)
          throw "missing required key intent for command `links_intent`";
        const release = intent.kind === "release" ? intent.reason : null;
        if (release === "rider_release" && started())
          throw err(
            "conflict",
            "the ride holds the trainer: end the ride first",
          );
        if (world.keeperStorage)
          throw err("storage", "cannot read the pairings");
        if (release !== null && world.supersedes && world.phase === "none") {
          world.supersedes = false;
          throw err("conflict", SUPERSEDED);
        }
        if (world.phase === "loaded") {
          if (release === "sleep" || release === "rider_release") {
            // discarded: the path of Cancel, its links released
            world.phase = "none";
            publish();
          }
        } else if (!started()) {
          if (
            world.quitting &&
            (intent.kind === "likely" || intent.kind === "retry")
          )
            throw err("conflict", "the application is quitting");
          if (release !== null) {
            world.keeper = {
              trainer: cold(world.keeper.trainer),
              strap: cold(world.keeper.strap),
            };
            show();
          }
        }
        return null;
      }
      case "ride_plan": {
        if (typeof args.rideId !== "string")
          throw "missing required key rideId for command `ride_plan`";
        if (args.rideId !== world.rideId)
          throw err("not_found", `no loaded ride "${args.rideId}"`);
        const totalS = world.blocks.reduce(
          (end, b) => Math.max(end, (b.startS ?? 0) + (b.durS ?? 0)),
          0,
        );
        // A free ride's plan is the core's free ride (core.rs `Core::resolve_workout`): no steps, at the snapshot's
        // FTP. The snapshot is the world's (a fresh store's by default: the priors of FTP, CP and W′).
        const free = world.workoutId === null;
        return {
          rideId: world.rideId,
          athlete: world.athlete,
          workout: free
            ? { id: "free-ride", name: "Free ride", ftp: 250, nodes: [] }
            : {
                id: world.workoutId,
                name: world.workoutName,
                ftp: 250,
                nodes: [],
              },
          compiled: {
            blocks: world.blocks,
            totalS,
            totalRangeS: [totalS, totalS],
            tss: 10,
            if: 0.7,
            timeInZoneS: [0, 0, 0, 0, 0, 0, 0],
            issues: [],
            ftpW: 250,
          },
        } as unknown as RidePlan;
      }
      case "workout_get": {
        if (typeof args.id !== "string")
          throw "missing required key id for command `workout_get`";
        if (args.id !== "w1") throw err("not_found", "unknown workout");
        // The compiled length is the end of the last block, as the core compiles it.
        const totalS = world.blocks.reduce(
          (end, b) => Math.max(end, (b.startS ?? 0) + (b.durS ?? 0)),
          0,
        );
        return {
          workout: { id: "w1", name: world.workoutName, ftp: 250, nodes: [] },
          dsl: "",
          compiled: {
            blocks: world.blocks,
            totalS,
            totalRangeS: [totalS, totalS],
            tss: 10,
            if: 0.7,
            timeInZoneS: [0, 0, 0, 0, 0, 0, 0],
            issues: [],
            ftpW: 250,
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
    "links_intent",
    "workout_get",
    "ride_plan",
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
  // The core's state at the start: the first frame of a channel opened now carries it.
  show();
  return {
    /** The snapshot channel (`ui_open`, `ui_ack`) and the series it reads. */
    ui: channel,
    world,
    calls,
    count: (cmd: string) => calls.filter((c) => c.cmd === cmd).length,
    publish,
    /** The reaper takes a ride that finished on its own: its hold is dropped and its devices come back (core.rs `Core::reap`). */
    reap(): void {
      world.unreaped = false;
      settleLinks();
      show();
    },
    /** The keeper's phases or samples change (warm-up, cool-down): the next frame carries them, as the core's would. */
    setKeeper(view: LinkView): void {
      world.keeper = view;
      show();
    },
    /** This tick's values (`ride.live`), sent as the next frame. */
    setLive(live: Partial<LiveValues>): void {
      world.live = { ...world.live, ...live };
      show();
    },
    /** The engine wedges (#351): the ride commands answer `engine_unresponsive`, and the frame says so. */
    wedge(): void {
      world.engineUnresponsive = true;
      show();
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
