// The core's workout commands behind `mockIPC` (`workouts_list`, `workout_get`, `ride_load`, `ride_status`), for the
// Workouts tests and the design fixtures (app/e2e/design/fixture.tsx, #457). Moved from Workouts.test.tsx unchanged.
import { mockIPC } from "@tauri-apps/api/mocks";
import type {
  CompiledView,
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
export const detail = (id: string, c: CompiledView): WorkoutDetail => ({
  workout: {
    id,
    name: SUMMARIES.find((s) => s.id === id)?.name ?? id,
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
  /** Per-command failures, thrown when the call arrives. */
  fail: Record<string, unknown>;
  /** The compiled view each workout answers with. */
  compiled: Record<string, CompiledView>;
  /** A ride is already in progress. */
  rideActive: boolean;
  /** A trainer is paired (core.rs `connect_links` (~1240)). */
  trainerPaired: boolean;
  /** The trainer does not answer within the connect timeout (core.rs `connect_links`, `CONNECT_TIMEOUT`). */
  connectTimesOut: boolean;
  /** `ride_load` calls currently in flight (the core's `loading` flag, core.rs `ride_load` (~1395)). */
  loading: number;
  /** Hold the next `ride_load` answer until the gate opens. */
  gate: Promise<void> | null;
  /** A load started elsewhere is in flight: `ride_status().loading` is true (core.rs `ride_status` (~1940)). */
  coreLoading: boolean;
  /** A refusal raised at one stage of `ride_load` (see the doc below), while the load is in flight. */
  stage: Partial<
    Record<"storage" | "driver" | "connect" | "build" | "installed", ApiErr>
  >;
}
export type ApiErr = { code: string; message: string };
/**
 * The core's workout commands behind `mockIPC`. `ride_load` refuses, in the core's order
 * (crates/crank-core/src/core.rs, symbols with `~line` hints):
 * - unknown id: `not_found` (`Library::get`, library.rs `get` (~75), via core.rs `resolve_workout` (~1349))
 * - error-severity issue: `invalid_input` with the issue's message (`resolve_workout`)
 * - ride in progress: `conflict` (`ride_load`, "a ride is in progress")
 * - another load in flight: `conflict` (core.rs `Core::ride_load` (~1395), "another ride is loading or loaded")
 * - `stage.storage`: `store.devices()?` fails, `storage` (`connect_links`)
 * - no trainer paired: `device_unavailable` (`connect_links`, "no trainer paired")
 * - `stage.driver`: `backend.trainer(..)?` fails, e.g. `permission_denied` or Bluetooth off (`connect_links`, error.rs `ApiError` (~90), its `From<DeviceError>`)
 * - trainer connect timeout: `device_unavailable` (`connect_links`, "trainer connect timed out")
 * - `stage.connect`: the inner connect error, e.g. `device_unavailable` "connect failed: .." (`connect_links`, the `??`)
 * - `stage.build`: `build_session` fails after connecting, e.g. `internal` "engine stopped" (core.rs `build_session` (~1563), `bounded(handle.send(EngineCmd::Load(..)))`)
 * - `stage.build`: the new engine does not take the ride within 1 s, `engine_unresponsive`, its devices released
 *   (core.rs `build_session` (~1563), core.rs `bounded` (~601) and core.rs `REQUEST_TIMEOUT` (~81), #351)
 * - `stage.installed`: `internal` "built session missing" (core.rs `ride_load` (~1395))
 * Everything from the storage step on happens while the core's `loading` flag is set (core.rs `ride_load` (~1395)), so the mock raises
 * those refusals after the load's delay (the gate), with the in-flight conflict active meanwhile.
 * (the strap driver `heart(..)?` in `connect_links` maps like `stage.driver`). core.rs `ensure_start_allowed` (~1236) always passes
 * today and is not modelled. A free ride (null id) skips the workout checks only. Tauri itself rejects a call
 * without its `workoutId` argument with a plain string.
 */
export function install(over: Partial<Backend> = {}): Backend {
  const b: Backend = {
    calls: [],
    fail: {},
    compiled: { "w-ss": compiled(), "w-easy": compiled({ tss: 20 }) },
    rideActive: false,
    trainerPaired: true,
    connectTimesOut: false,
    loading: 0,
    gate: null,
    coreLoading: false,
    stage: {},
    ...over,
  };
  mockIPC((cmd, payload) => {
    const args = (payload ?? {}) as Record<string, unknown>;
    b.calls.push({ cmd, args });
    if (cmd in b.fail) throw b.fail[cmd];
    switch (cmd) {
      case "workouts_list":
        return SUMMARIES;
      case "workout_get": {
        const id = args.id as string;
        const c = b.compiled[id];
        if (!c) throw apiError("not_found", `no workout ${id}`);
        return detail(id, c);
      }
      case "ride_load": {
        if (!("workoutId" in args))
          throw "missing required key workoutId for command `ride_load`";
        const id = args.workoutId as string | null;
        if (id !== null) {
          const c = b.compiled[id];
          if (!c) throw apiError("not_found", `no workout ${id}`);
          const issue = c.issues.find((i) => i.severity === "error");
          if (issue) throw apiError("invalid_input", issue.message);
        }
        if (b.rideActive) throw apiError("conflict", "a ride is in progress");
        if (b.loading > 0)
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
          loading: b.loading > 0 || b.coreLoading,
          status: idleStatus(),
        };
      default:
        throw `command ${cmd} not found`;
    }
  });
  return b;
}
