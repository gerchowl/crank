// The core's workout commands behind `mockIPC` (`workouts_list`, `workout_get`, `ride_load`, `ride_status`), for the
// Workouts tests and the design fixtures (app/e2e/design/fixture.tsx, #457).
import { mockIPC } from "@tauri-apps/api/mocks";
import type {
  CompiledView,
  LoadProgress,
  RideLoaded,
  WorkoutDetail,
  WorkoutIssue,
  WorkoutSummary,
} from "../bindings";
import { status as idleStatus } from "../stream/testBackend";
export const apiError = (code: string, message: string) => ({ code, message });

export const SUMMARIES: WorkoutSummary[] = [
  { id: "w-ss", name: "Sweet spot 3x12", totalS: 3000, tss: 52.4, ftp: 250 },
  { id: "w-easy", name: "Easy spin", totalS: 1800, tss: 20, ftp: 250 },
];
export const compiled = (over: Partial<CompiledView> = {}): CompiledView => ({
  blocks: [],
  totalS: 3000,
  totalRangeS: [2900, 3300],
  tss: 52.4,
  if: 0.78,
  timeInZoneS: [300, 1200, 900, 600, 30, 120, 3661],
  issues: [],
  ...over,
});
export const detail = (
  id: string,
  c: CompiledView,
  summaries: readonly WorkoutSummary[] = SUMMARIES,
): WorkoutDetail => ({
  workout: {
    id,
    name: summaries.find((s) => s.id === id)?.name ?? id,
    ftp: 250,
    nodes: [],
  },
  dsl: "",
  compiled: c,
});
export const ERROR_ISSUE: WorkoutIssue = {
  kind: "invalid_ftp",
  severity: "error",
  nodeId: null,
  message: "FTP is not positive",
};
export const WARNING: WorkoutIssue = {
  kind: "low_reserve",
  severity: "warning",
  nodeId: null,
  message: "W′ reserve drops below 15 percent",
};

export interface Backend {
  calls: { cmd: string; args: Record<string, unknown> }[];
  /**
   * Per-command failures, thrown when the call arrives. `workouts_list` and `ride_status` never fail in the core today
   * (core.rs `Core::workouts_list` (~1480), core.rs `Core::ride_status` (~2382)); a failure of either is the store-backed
   * library to come (#110) or Tauri's own rejection, which the view shows all the same.
   */
  fail: Record<string, unknown>;
  /**
   * The library, in its order (the built-in library is never empty today; the store-backed one of #110 can be). It is
   * the one library `workouts_list`, `workout_get` and `ride_load` all read (library.rs `Library::summaries` (~81),
   * library.rs `Library::get` (~75)): an id not in it is `not_found` for every command.
   */
  summaries: WorkoutSummary[];
  /** The compiled view each workout of `summaries` answers with (a listed id without one is a broken fixture). */
  compiled: Record<string, CompiledView>;
  /** A ride is already in progress. */
  rideActive: boolean;
  /** A trainer is paired (core.rs `connect_links` (~1515)). */
  trainerPaired: boolean;
  /** The trainer does not answer within the connect timeout (run.rs `Task::attempt`, core.rs `CONNECT_TIMEOUT`). */
  connectTimesOut: boolean;
  /** `ride_load` calls currently in flight (the core's `loading` flag, core.rs `ride_load` (~1679)). */
  loading: number;
  /** Hold the next `ride_load` answer until the gate opens. */
  gate: Promise<void> | null;
  /** A load started elsewhere is in flight: `ride_status().load` is set (core.rs `ride_status` (~2382)). */
  coreLoading: boolean;
  /** The phases `ride_status().load` reports while a load is in flight (the keeper's, core.rs `Core::ride_status`). */
  progress: LoadProgress;
  /** A refusal raised at one stage of `ride_load` (see the doc below), while the load is in flight. */
  stage: Partial<
    Record<"storage" | "driver" | "connect" | "build" | "installed", ApiErr>
  >;
}
export type ApiErr = { code: string; message: string };
/**
 * The core's workout commands behind `mockIPC`, each refusing as the core does (crates/crank-core/src, symbols with
 * `~line` hints):
 * - `workouts_list` never fails (core.rs `Core::workouts_list` (~1480)); `fail` injects one (see `Backend.fail`).
 * - `workout_get`: unknown id, `not_found` (library.rs `Library::detail` (~93), library.rs `Library::get` (~75)). Its
 *   other refusal, `internal` from a block index that does not fit (dto.rs `WorkoutBlock` (~196)), is unreachable
 *   through the API and not modelled.
 * - `ride_status` never fails (core.rs `Core::ride_status` (~2382)). Its `load` is set while the core's one flag is (a
 *   load in flight here, or `coreLoading`), with `progress` as the phases; its `status` is always idle, as no test or
 *   fixture here reads it.
 *
 * `ride_load` refuses, in the core's order (core.rs `Core::ride_load` (~1679)):
 * - unknown id: `not_found` (library.rs `Library::get` (~75), via core.rs `Core::resolve_workout` (~1633))
 * - error-severity issue: `invalid_input` with the issue's message (`resolve_workout`)
 * - ride in progress: `conflict` (`ride_load`, "a ride is in progress")
 * - another load in flight, here or started by another window (`coreLoading`): `conflict`
 *   (core.rs `Core::ride_load` (~1679), "another ride is loading or loaded")
 * - `stage.storage`: `store.devices()?` fails, `storage` (core.rs `Core::connect_links` (~1515), or the keeper's start
 *   reading the pairings, core.rs `Core::pairings` (~1201))
 * - no trainer paired: `device_unavailable` (`connect_links`, "no trainer paired")
 * - `stage.driver`: the trainer driver fails before connecting, e.g. `permission_denied` or Bluetooth off (the link
 *   keeper's attempt, run.rs `Task::attempt` (~545), `devices.trainer()`; error.rs `ApiError` (~90), its `From<DeviceError>`)
 * - trainer connect timeout: `device_unavailable` "trainer connect timed out" (run.rs `Task::attempt`, `timed_out`)
 * - `stage.connect`: any other failure of the keeper's `acquire` (run.rs `Handle::acquire` (~180)): the inner connect
 *   error, e.g. `device_unavailable` "connect failed: .."; `internal` "the trainer connect panicked" (run.rs
 *   `Task::attempt`); `internal` "the warm trainer link was gone at the handover" (run.rs `Task::hand_over` (~688));
 *   `device_unavailable` "the trainer was released" (run.rs `Task::fail_load` (~731)); `internal` "the link keeper is
 *   gone" (`acquire`)
 * - `stage.build`: `build_session` fails after connecting, e.g. `internal` "engine stopped" (core.rs `Core::build_session` (~1901), `bounded(handle.send(EngineCmd::Load(..)))`)
 * - `stage.build`: the new engine does not take the ride within 1 s, `engine_unresponsive`, its devices released
 *   (core.rs `Core::build_session` (~1901), core.rs `bounded` (~676) and core.rs `REQUEST_TIMEOUT` (~84), #351)
 * - `stage.build`: a quit while the load connects or builds, `conflict` "the application is quitting" (`ride_load`)
 * - `stage.installed`: `internal` "built session missing" (core.rs `Core::ride_load` (~1679))
 * Everything from the storage step on happens while the core's `loading` flag is set (`ride_load`), so the mock raises
 * those refusals after the load's delay (the gate), with the in-flight conflict active meanwhile.
 * (The strap's failures never refuse a load: the ride runs without heart rate, `connect_links`.)
 * core.rs `Core::ensure_start_allowed` (~1505) always passes today and is not modelled. A free ride (null id) skips the
 * workout checks only. Tauri itself rejects a call without its `workoutId` argument with a plain string.
 */
export function install(over: Partial<Backend> = {}): Backend {
  const b: Backend = {
    calls: [],
    fail: {},
    summaries: SUMMARIES,
    compiled: { "w-ss": compiled(), "w-easy": compiled({ tss: 20 }) },
    rideActive: false,
    trainerPaired: true,
    connectTimesOut: false,
    loading: 0,
    gate: null,
    coreLoading: false,
    progress: { trainer: "searching", strap: null },
    stage: {},
    ...over,
  };
  /** The library's entry for `id`: `not_found` with the core's text (`{id:?}` quotes it) when it is not listed. */
  const compiledOf = (id: string): CompiledView => {
    if (!b.summaries.some((w) => w.id === id))
      throw apiError("not_found", `no workout "${id}"`);
    const c = b.compiled[id];
    if (!c)
      throw new Error(
        `fixture: workout ${id} is listed but has no compiled view`,
      );
    return c;
  };
  mockIPC((cmd, payload) => {
    const args = (payload ?? {}) as Record<string, unknown>;
    b.calls.push({ cmd, args });
    if (cmd in b.fail) throw b.fail[cmd];
    switch (cmd) {
      case "workouts_list":
        return b.summaries;
      case "workout_get": {
        const id = args.id as string;
        return detail(id, compiledOf(id), b.summaries);
      }
      case "ride_load": {
        if (!("workoutId" in args))
          throw "missing required key workoutId for command `ride_load`";
        const id = args.workoutId as string | null;
        if (id !== null) {
          const issue = compiledOf(id).issues.find(
            (i) => i.severity === "error",
          );
          if (issue) throw apiError("invalid_input", issue.message);
        }
        if (b.rideActive) throw apiError("conflict", "a ride is in progress");
        // One `loading` flag for the whole core, whichever window started the load (core.rs `Core::ride_load` (~1679),
        // `loading.swap(true)`): a load this mock runs, or one started elsewhere (`coreLoading`).
        if (b.loading > 0 || b.coreLoading)
          throw apiError("conflict", "another ride is loading or loaded");
        const done = () =>
          ({
            rideId: "ride-1",
            workoutId: id,
            hrLinked: false,
          }) satisfies RideLoaded;
        // From here the core's `loading` flag is set: the connect, the build and their refusals take time.
        b.loading++;
        const load = async () => {
          try {
            const gate = b.gate;
            b.gate = null;
            if (gate) await gate;
            if (b.stage.storage) throw b.stage.storage;
            if (!b.trainerPaired)
              throw apiError("device_unavailable", "no trainer paired");
            if (b.stage.driver) throw b.stage.driver;
            if (b.connectTimesOut)
              throw apiError("device_unavailable", "trainer connect timed out");
            if (b.stage.connect) throw b.stage.connect;
            if (b.stage.build) throw b.stage.build;
            if (b.stage.installed) throw b.stage.installed;
            return done();
          } finally {
            b.loading--;
          }
        };
        return load();
      }
      case "ride_status":
        return {
          load: b.loading > 0 || b.coreLoading ? b.progress : null,
          status: idleStatus(),
        };
      default:
        throw `command ${cmd} not found`;
    }
  });
  return b;
}
