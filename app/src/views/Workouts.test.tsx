// @vitest-environment jsdom
import {
  act,
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach, expect, test } from "vitest";
import type {
  CompiledView,
  RideLoaded,
  WorkoutDetail,
  WorkoutIssue,
  WorkoutSummary,
} from "../bindings";
import { navigate, useRoute } from "../router";
import { currentSession } from "../ride/session";
import { Workouts } from "./Workouts";
import { RideLoadNotice } from "./RideLoadNotice";
import { resetRideLoad, startRideLoad } from "./rideLoad";
import { status as idleStatus } from "../stream/testBackend";

afterEach(() => {
  cleanup();
  clearMocks();
});
beforeEach(() => {
  resetRideLoad();
  navigate("workouts");
});

const apiError = (code: string, message: string) => ({ code, message });

const SUMMARIES: WorkoutSummary[] = [
  { id: "w-ss", name: "Sweet spot 3x12", totalS: 3000, tss: 52.4, ftp: 250 },
  { id: "w-easy", name: "Easy spin", totalS: 1800, tss: 20, ftp: 250 },
];
const compiled = (over: Partial<CompiledView> = {}): CompiledView => ({
  blocks: [],
  totalS: 3000,
  totalRangeS: [2900, 3300],
  tss: 52.4,
  if: 0.78,
  timeInZoneS: [300, 1200, 900, 600, 30, 120, 3661],
  issues: [],
  ...over,
});
const detail = (id: string, c: CompiledView): WorkoutDetail => ({
  workout: {
    id,
    name: SUMMARIES.find((s) => s.id === id)?.name ?? id,
    ftp: 250,
    nodes: [],
  },
  dsl: "",
  compiled: c,
});
const ERROR_ISSUE: WorkoutIssue = {
  kind: "invalid_ftp",
  severity: "error",
  nodeId: null,
  message: "FTP is not positive",
};
const WARNING: WorkoutIssue = {
  kind: "low_reserve",
  severity: "warning",
  nodeId: null,
  message: "W′ reserve drops below 15 percent",
};

interface Backend {
  calls: { cmd: string; args: Record<string, unknown> }[];
  /** Per-command failures, thrown when the call arrives. */
  fail: Record<string, unknown>;
  /** The compiled view each workout answers with. */
  compiled: Record<string, CompiledView>;
  /** A ride is already in progress. */
  rideActive: boolean;
  /** A trainer is paired (core.rs:1184-1185, `connect_links`). */
  trainerPaired: boolean;
  /** The trainer does not answer within the connect timeout (core.rs:1188-1193). */
  connectTimesOut: boolean;
  /** `ride_load` calls currently in flight (the core's `loading` flag, core.rs:1379). */
  loading: number;
  /** Hold the next `ride_load` answer until the gate opens. */
  gate: Promise<void> | null;
  /** A load started elsewhere is in flight: `ride_status().loading` is true (core.rs:1850). */
  coreLoading: boolean;
  /** A refusal raised at one stage of `ride_load` (see the doc below), while the load is in flight. */
  stage: Partial<
    Record<"storage" | "driver" | "connect" | "build" | "installed", ApiErr>
  >;
}
type ApiErr = { code: string; message: string };
/**
 * The core's workout commands behind `mockIPC`. `ride_load` refuses, in the core's order
 * (crates/crank-core/src/core.rs, line numbers at #380):
 * - unknown id: `not_found` (`Library::get`, library.rs:75-79, via `resolve_workout` :1288, :1307)
 * - error-severity issue: `invalid_input` with the issue's message (:1309-1318)
 * - ride in progress: `conflict` (:1347-1352)
 * - another load in flight: `conflict` (:1379-1384)
 * - `stage.storage`: `store.devices()?` fails, `storage` (:1182, `connect_links`)
 * - no trainer paired: `device_unavailable` (:1184-1185)
 * - `stage.driver`: `backend.trainer(..)?` fails, e.g. `permission_denied` or Bluetooth off (:1187, error.rs:90-117)
 * - trainer connect timeout: `device_unavailable` (:1188-1193)
 * - `stage.connect`: the inner connect error, e.g. `device_unavailable` "connect failed: .." (:1193 `??`)
 * - `stage.build`: `build_session` fails after connecting, e.g. `internal` "engine stopped" (:1537, `bounded(handle.send(EngineCmd::Load(..)))`)
 * - `stage.build`: the new engine does not take the ride within 1 s, `engine_unresponsive`, its devices released
 *   (`build_session`, `bounded` and `REQUEST_TIMEOUT` in core.rs, #351)
 * - `stage.installed`: `internal` "built session missing" (:1399)
 * Everything from the storage step on happens while the core's `loading` flag is set (:1379), so the mock raises
 * those refusals after the load's delay (the gate), with the in-flight conflict active meanwhile.
 * (the strap driver `heart(..)?` at :1197 maps like `stage.driver`). `ensure_start_allowed` (:1175) always passes
 * today and is not modelled. A free ride (null id) skips the workout checks only. Tauri itself rejects a call
 * without its `workoutId` argument with a plain string.
 */
function install(over: Partial<Backend> = {}): Backend {
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

function Routed() {
  const route = useRoute();
  return (
    <>
      <p data-testid="route">{route.view}</p>
      {route.view === "workouts" && <Workouts />}
      <RideLoadNotice />
    </>
  );
}
const open = () => render(<Routed />);
const summary = () => screen.findByRole("region", { name: "Compiled summary" });

test("the library list renders from the core's workouts command", async () => {
  install();
  open();
  expect(
    await screen.findByRole("button", { name: /Sweet spot 3x12/ }),
  ).toBeTruthy();
  expect(screen.getByRole("button", { name: /Easy spin/ })).toBeTruthy();
  // list rows carry the summary's duration and stress
  const row = screen.getByRole("button", { name: /Sweet spot 3x12/ });
  expect(row.textContent).toMatch(/50:00/);
  expect(row.textContent).toMatch(/TSS 52/);
});

test("picking a workout shows the compiled summary from the core", async () => {
  const b = install();
  open();
  await userEvent.click(
    await screen.findByRole("button", { name: /Sweet spot 3x12/ }),
  );
  const s = within(await summary());
  expect(b.calls.find((c) => c.cmd === "workout_get")?.args).toEqual({
    id: "w-ss",
  });
  expect(await s.findByText("48:20–55:00")).toBeTruthy(); // duration range
  expect(s.getByText("52")).toBeTruthy(); // TSS
  expect(s.getByText("0.78")).toBeTruthy(); // IF
  // zone time: seven zones, each in m:ss
  for (const [zone, t] of [
    ["Z1", "5:00"],
    ["Z2", "20:00"],
    ["Z3", "15:00"],
    ["Z4", "10:00"],
    ["Z5", "0:30"],
    ["Z6", "2:00"],
    ["Z7", "1:01:01"],
  ] as const) {
    const row = s.getByText(zone).closest("li")!;
    expect(row.textContent).toContain(t);
  }
  expect(s.getAllByRole("listitem").length).toBeGreaterThanOrEqual(7);
});

test("a validation warning renders with its message", async () => {
  install({
    compiled: {
      "w-ss": compiled({ issues: [WARNING] }),
      "w-easy": compiled(),
    },
  });
  open();
  await userEvent.click(
    await screen.findByRole("button", { name: /Sweet spot 3x12/ }),
  );
  const s = within(await summary());
  const issue = await s.findByText(/W′ reserve drops below 15 percent/);
  expect(issue.closest("li")!.textContent).toMatch(/Warning/);
});

test("Ride this loads the picked workout and routes to the ride", async () => {
  const b = install();
  open();
  await userEvent.click(
    await screen.findByRole("button", { name: /Easy spin/ }),
  );
  await summary();
  await userEvent.click(
    await screen.findByRole("button", { name: "Ride this" }),
  );
  await waitFor(() =>
    expect(screen.getByTestId("route").textContent).toBe("ride"),
  );
  expect(b.calls.find((c) => c.cmd === "ride_load")?.args).toEqual({
    workoutId: "w-easy",
  });
  // The ride screen's session is the one ride_load made.
  expect(currentSession()?.workoutId).toBe("w-easy");
  expect(currentSession()?.rideId).toBe("ride-1");
});

test("a failing library list shows the error and can be retried", async () => {
  const b = install({
    fail: { workouts_list: apiError("storage", "disk gone") },
  });
  open();
  expect((await screen.findByRole("alert")).textContent).toMatch(/disk gone/);
  delete b.fail.workouts_list;
  await userEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(
    await screen.findByRole("button", { name: /Sweet spot 3x12/ }),
  ).toBeTruthy();
  expect(screen.queryByRole("alert")).toBeNull();
});

test("a workout the core cannot give shows the error instead of a summary", async () => {
  install({ compiled: { "w-easy": compiled() } }); // w-ss is listed but gone
  open();
  await userEvent.click(
    await screen.findByRole("button", { name: /Sweet spot 3x12/ }),
  );
  expect((await screen.findByRole("alert")).textContent).toMatch(
    /no workout w-ss/,
  );
  expect(screen.queryByRole("region", { name: "Compiled summary" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Ride this" })).toBeNull();
});

test("a refused ride-load shows the core's reason and stays on the page", async () => {
  install({ rideActive: true });
  open();
  await userEvent.click(
    await screen.findByRole("button", { name: /Sweet spot 3x12/ }),
  );
  await summary();
  await userEvent.click(
    await screen.findByRole("button", { name: "Ride this" }),
  );
  expect((await screen.findByRole("alert")).textContent).toMatch(
    /ride is in progress/,
  );
  expect(screen.getByTestId("route").textContent).toBe("workouts");
});

test("a load the core refuses for an error-severity issue shows its message and stays", async () => {
  // The library changed between the pick and the click: the core re-checks at load (core.rs:1288-1319, `resolve_workout`).
  const b = install();
  open();
  await userEvent.click(
    await screen.findByRole("button", { name: /Sweet spot 3x12/ }),
  );
  await summary();
  b.compiled["w-ss"] = compiled({ issues: [ERROR_ISSUE] });
  await userEvent.click(
    await screen.findByRole("button", { name: "Ride this" }),
  );
  expect((await screen.findByRole("alert")).textContent).toMatch(
    /FTP is not positive/,
  );
  expect(screen.getByTestId("route").textContent).toBe("workouts");
});

test("with no trainer paired the load says so, points to Settings → Devices and stays", async () => {
  install({ trainerPaired: false });
  open();
  await userEvent.click(
    await screen.findByRole("button", { name: /Easy spin/ }),
  );
  await summary();
  await userEvent.click(
    await screen.findByRole("button", { name: "Ride this" }),
  );
  const alert = (await screen.findByRole("alert")).textContent!;
  expect(alert).toMatch(/no trainer paired/);
  expect(alert).toMatch(/Settings → Devices/);
  expect(screen.getByTestId("route").textContent).toBe("workouts");
});

test("a trainer that does not answer is refused with the core's message and stays", async () => {
  install({ connectTimesOut: true });
  open();
  await userEvent.click(
    await screen.findByRole("button", { name: /Easy spin/ }),
  );
  await summary();
  await userEvent.click(
    await screen.findByRole("button", { name: "Ride this" }),
  );
  expect((await screen.findByRole("alert")).textContent).toMatch(
    /trainer connect timed out/,
  );
  expect(screen.getByTestId("route").textContent).toBe("workouts");
});

test("a workout with an error-severity issue cannot be ridden, and the reason is visible", async () => {
  const b = install({
    compiled: {
      "w-ss": compiled({ issues: [ERROR_ISSUE] }),
      "w-easy": compiled(),
    },
  });
  open();
  await userEvent.click(
    await screen.findByRole("button", { name: /Sweet spot 3x12/ }),
  );
  await summary();
  const ride = screen.getByRole("button", { name: "Ride this" });
  expect((ride as HTMLButtonElement).disabled).toBe(true);
  const reason = document.getElementById(
    ride.getAttribute("aria-describedby")!,
  )!;
  expect(reason.textContent).toMatch(/FTP is not positive/);
  await userEvent.click(ride);
  expect(b.calls.some((c) => c.cmd === "ride_load")).toBe(false);
});

test("a warning alone does not block riding", async () => {
  install({
    compiled: { "w-ss": compiled({ issues: [WARNING] }), "w-easy": compiled() },
  });
  open();
  await userEvent.click(
    await screen.findByRole("button", { name: /Sweet spot 3x12/ }),
  );
  await summary();
  expect(
    (screen.getByRole("button", { name: "Ride this" }) as HTMLButtonElement)
      .disabled,
  ).toBe(false);
});

test("a stale refusal is cleared when another workout is picked", async () => {
  install({ trainerPaired: false });
  open();
  await userEvent.click(
    await screen.findByRole("button", { name: /Easy spin/ }),
  );
  await summary();
  await userEvent.click(
    await screen.findByRole("button", { name: "Ride this" }),
  );
  await screen.findByRole("alert");
  await userEvent.click(
    screen.getByRole("button", { name: /Sweet spot 3x12/ }),
  );
  await summary();
  await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
});

test("a double click sends one ride_load while it is in flight", async () => {
  const b = install();
  let release = () => {};
  b.gate = new Promise<void>((resolve) => (release = resolve));
  open();
  await userEvent.click(
    await screen.findByRole("button", { name: /Easy spin/ }),
  );
  await summary();
  await userEvent.dblClick(
    await screen.findByRole("button", { name: "Ride this" }),
  );
  expect(b.calls.filter((c) => c.cmd === "ride_load")).toHaveLength(1);
  release();
  await waitFor(() =>
    expect(screen.getByTestId("route").textContent).toBe("ride"),
  );
  expect(screen.queryByRole("alert")).toBeNull();
});

// ---- #320: the selection is locked while a load is in flight ----

const pressed = (name: RegExp) =>
  screen.getByRole("button", { name }).getAttribute("aria-pressed");
const startHeldLoad = async (b: Backend, first: RegExp) => {
  let release = () => {};
  b.gate = new Promise<void>((resolve) => (release = resolve));
  open();
  await userEvent.click(await screen.findByRole("button", { name: first }));
  await summary();
  await userEvent.click(
    await screen.findByRole("button", { name: "Ride this" }),
  );
  await screen.findByText("Loading ride…");
  return release;
};

test("picking another workout while a load is in flight is not possible, and the load still routes for the first", async () => {
  const b = install();
  const release = await startHeldLoad(b, /Easy spin/);
  // the whole list is locked, with a visible reason
  for (const name of [/Easy spin/, /Sweet spot 3x12/])
    expect(
      (screen.getByRole("button", { name }) as HTMLButtonElement).disabled,
    ).toBe(true);
  await userEvent.click(
    screen.getByRole("button", { name: /Sweet spot 3x12/ }),
  );
  expect(pressed(/Easy spin/)).toBe("true");
  expect(pressed(/Sweet spot 3x12/)).toBe("false");
  expect(b.calls.filter((c) => c.cmd === "workout_get")).toHaveLength(1);

  release();
  await waitFor(() =>
    expect(screen.getByTestId("route").textContent).toBe("ride"),
  );
  // the ride that was loaded is the one that was picked when it was asked for
  expect(
    b.calls.filter((c) => c.cmd === "ride_load").map((c) => c.args),
  ).toEqual([{ workoutId: "w-easy" }]);
});

test("a refusal that arrives after a pick attempt appears under the workout it was for, and the list unlocks", async () => {
  const b = install();
  b.stage.connect = apiError(
    "device_unavailable",
    "connect failed: out of range",
  );
  const release = await startHeldLoad(b, /Easy spin/);
  await userEvent.click(
    screen.getByRole("button", { name: /Sweet spot 3x12/ }),
  );
  expect(pressed(/Easy spin/)).toBe("true");

  release();
  const alert = await screen.findByRole("alert");
  expect(alert.textContent).toMatch(/connect failed: out of range/);
  // still the first workout's page: its name is the heading the refusal sits under
  expect(screen.getByRole("heading", { name: "Easy spin" })).toBeTruthy();
  expect(pressed(/Easy spin/)).toBe("true");
  expect(screen.queryByText("Loading ride…")).toBeNull();
  expect(
    (
      screen.getByRole("button", {
        name: /Sweet spot 3x12/,
      }) as HTMLButtonElement
    ).disabled,
  ).toBe(false);
  expect(screen.getByTestId("route").textContent).toBe("workouts");
});

const REFUSALS: [string, Partial<Backend["stage"]>, RegExp][] = [
  [
    "storage",
    { storage: apiError("storage", "database is locked") },
    /database is locked/,
  ],
  [
    "the trainer driver",
    {
      driver: apiError(
        "permission_denied",
        "Bluetooth access is denied. Open System Settings",
      ),
    },
    /Bluetooth access is denied/,
  ],
  [
    "the inner connect",
    { connect: apiError("device_unavailable", "connect failed: out of range") },
    /connect failed: out of range/,
  ],
  [
    "build_session",
    { build: apiError("internal", "engine stopped") },
    /engine stopped/,
  ],
  [
    "an engine that does not take the ride",
    {
      build: apiError(
        "engine_unresponsive",
        "the ride engine did not take the ride in time; its devices were released, try again",
      ),
    },
    /did not take the ride in time/,
  ],
  [
    "the session install",
    { installed: apiError("internal", "built session missing") },
    /built session missing/,
  ],
];
for (const [where, stage, message] of REFUSALS)
  test(`a refusal from ${where} shows the core's message and stays on the page`, async () => {
    install({ stage });
    open();
    await userEvent.click(
      await screen.findByRole("button", { name: /Easy spin/ }),
    );
    await summary();
    await userEvent.click(
      await screen.findByRole("button", { name: "Ride this" }),
    );
    expect((await screen.findByRole("alert")).textContent).toMatch(message);
    expect(screen.getByTestId("route").textContent).toBe("workouts");
  });

// ---- #320 round 2: the load belongs to the app, not to the view ----

const leave = () => act(() => navigate("settings"));
const comeBack = () => act(() => navigate("workouts"));

test("leaving Workouts mid-load and coming back is still locked, and the load routes into the right ride", async () => {
  const b = install();
  const release = await startHeldLoad(b, /Easy spin/);
  await leave();
  await comeBack();
  // the view is new, the lock and its reason are not gone
  expect(await screen.findByText("Loading ride…")).toBeTruthy();
  for (const name of [/Easy spin/, /Sweet spot 3x12/])
    expect(
      (await screen.findByRole("button", { name })) as HTMLButtonElement,
    ).toHaveProperty("disabled", true);
  await userEvent.click(
    screen.getByRole("button", { name: /Sweet spot 3x12/ }),
  );
  expect(b.calls.filter((c) => c.cmd === "workout_get")).toHaveLength(1);

  release();
  await waitFor(() =>
    expect(screen.getByTestId("route").textContent).toBe("ride"),
  );
  expect(currentSession()?.workoutId).toBe("w-easy");
  expect(b.calls.filter((c) => c.cmd === "ride_load")).toHaveLength(1);
});

test("a load the core reports as in flight locks a freshly mounted view, and unlocks when it clears", async () => {
  const b = install({ coreLoading: true });
  open();
  expect(await screen.findByText("Loading ride…")).toBeTruthy();
  expect(
    (await screen.findByRole("button", {
      name: /Easy spin/,
    })) as HTMLButtonElement,
  ).toHaveProperty("disabled", true);
  b.coreLoading = false;
  await waitFor(() => expect(screen.queryByText("Loading ride…")).toBeNull());
  expect(
    (screen.getByRole("button", { name: /Easy spin/ }) as HTMLButtonElement)
      .disabled,
  ).toBe(false);
});

test("a load that completes while the rider is elsewhere does not pull them out; the notice offers the ride", async () => {
  const b = install();
  const release = await startHeldLoad(b, /Easy spin/);
  await leave();
  release();
  const notice = await screen.findByText(/Ride loaded: Easy spin/);
  expect(screen.getByTestId("route").textContent).toBe("settings");
  expect(currentSession()?.workoutId).toBe("w-easy");
  await userEvent.click(
    within(notice.closest("p")!).getByRole("button", { name: "Go to ride" }),
  );
  expect(screen.getByTestId("route").textContent).toBe("ride");
  expect(screen.queryByText(/Ride loaded/)).toBeNull();
});

test("the loaded-ride notice can be dismissed", async () => {
  const b = install();
  const release = await startHeldLoad(b, /Easy spin/);
  await leave();
  release();
  await screen.findByText(/Ride loaded: Easy spin/);
  await userEvent.click(screen.getByRole("button", { name: "Dismiss" }));
  expect(screen.queryByText(/Ride loaded/)).toBeNull();
  expect(screen.getByTestId("route").textContent).toBe("settings");
});

test("the loaded-ride notice goes away when the rider opens the ride another way", async () => {
  const b = install();
  const release = await startHeldLoad(b, /Easy spin/);
  await leave();
  release();
  await screen.findByText(/Ride loaded: Easy spin/);
  await act(() => navigate("ride"));
  await waitFor(() => expect(screen.queryByText(/Ride loaded/)).toBeNull());
});

test("a refusal that arrives while the rider is elsewhere is shown in the notice, not under the wrong workout", async () => {
  const b = install();
  b.stage.connect = apiError(
    "device_unavailable",
    "connect failed: out of range",
  );
  const release = await startHeldLoad(b, /Easy spin/);
  await leave();
  release();
  const alert = await screen.findByRole("alert");
  expect(alert.textContent).toMatch(/Could not load Easy spin/);
  expect(alert.textContent).toMatch(/connect failed: out of range/);
  expect(screen.getByTestId("route").textContent).toBe("settings");
  await userEvent.click(within(alert).getByRole("button", { name: "Dismiss" }));
  expect(screen.queryByRole("alert")).toBeNull();
  // back on the list nothing is stale, nothing is locked
  await comeBack();
  expect(
    (
      (await screen.findByRole("button", {
        name: /Sweet spot 3x12/,
      })) as HTMLButtonElement
    ).disabled,
  ).toBe(false);
  expect(screen.queryByRole("alert")).toBeNull();
});

// ---- #333: a refusal is never dropped, and shown once ----

const CONNECT_FAILED = apiError(
  "device_unavailable",
  "connect failed: out of range",
);

test("a refusal after leave-and-return is shown on Workouts naming the workout, and not again elsewhere", async () => {
  const b = install();
  b.stage.connect = CONNECT_FAILED;
  const release = await startHeldLoad(b, /Easy spin/);
  await leave();
  await comeBack();
  expect(await screen.findByText("Loading ride…")).toBeTruthy();
  release();
  const alert = await screen.findByRole("alert");
  expect(alert.textContent).toMatch(/Could not load Easy spin/);
  expect(alert.textContent).toMatch(/connect failed: out of range/);
  await waitFor(() => expect(screen.queryByText("Loading ride…")).toBeNull());
  await leave();
  expect(screen.queryByRole("alert")).toBeNull();
});

test("a refusal that arrives elsewhere is shown there, once on return, then cleared", async () => {
  const b = install();
  b.stage.connect = CONNECT_FAILED;
  const release = await startHeldLoad(b, /Easy spin/);
  await leave();
  release();
  expect((await screen.findByRole("alert")).textContent).toMatch(
    /Could not load Easy spin/,
  );
  await comeBack(); // not dismissed
  const onWorkouts = await screen.findByRole("alert");
  expect(onWorkouts.textContent).toMatch(/Could not load Easy spin/);
  expect(onWorkouts.textContent).toMatch(/connect failed: out of range/);
  await leave();
  expect(screen.queryByRole("alert")).toBeNull();
  await comeBack();
  expect(
    await screen.findByRole("button", { name: /Sweet spot 3x12/ }),
  ).toBeTruthy();
  expect(screen.queryByRole("alert")).toBeNull();
});

test("a refusal shown on Workouts can be dismissed", async () => {
  const b = install();
  b.stage.connect = CONNECT_FAILED;
  const release = await startHeldLoad(b, /Easy spin/);
  await leave();
  await comeBack();
  release();
  const alert = await screen.findByRole("alert");
  await userEvent.click(within(alert).getByRole("button", { name: "Dismiss" }));
  expect(screen.queryByRole("alert")).toBeNull();
});

// ---- #338: what `seenRideRefusal` leaves alone, and a refusal that arrives on the ride screen ----

test("leaving Workouts does not clear a loaded result: only a refusal is seen", async () => {
  const b = install();
  const release = await startHeldLoad(b, /Easy spin/);
  await leave();
  release();
  await screen.findByText(/Ride loaded: Easy spin/);
  await comeBack();
  await leave(); // Workouts left again: `seenRideRefusal` runs, and must keep the loaded result
  expect(screen.getByText(/Ride loaded: Easy spin/)).toBeTruthy();
  expect(screen.getByRole("button", { name: "Go to ride" })).toBeTruthy();
});

test("a refusal that arrives on the ride screen is hidden there and shown on the first view after it", async () => {
  const b = install();
  b.stage.connect = CONNECT_FAILED;
  const release = await startHeldLoad(b, /Easy spin/);
  await leave();
  await act(() => navigate("ride")); // the rider opened the ride another way; the load is still in flight
  release();
  await waitFor(() =>
    expect(b.calls.filter((c) => c.cmd === "ride_load")).toHaveLength(1),
  );
  await act(async () => {}); // let the refusal settle
  expect(screen.queryByRole("alert")).toBeNull();
  await act(() => navigate("today"));
  const alert = await screen.findByRole("alert");
  expect(alert.textContent).toMatch(/Could not load Easy spin/);
  expect(alert.textContent).toMatch(/connect failed: out of range/);
});

test("starting a load while one is in flight is a no-op at the controller", async () => {
  const b = install();
  let release = () => {};
  b.gate = new Promise<void>((resolve) => (release = resolve));
  const first = startRideLoad("w-easy", "Easy spin");
  await startRideLoad("w-ss", "Sweet spot 3x12");
  expect(b.calls.filter((c) => c.cmd === "ride_load")).toHaveLength(1);
  release();
  await first;
});
