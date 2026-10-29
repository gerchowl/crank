// @vitest-environment jsdom
import {
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

afterEach(() => {
  cleanup();
  clearMocks();
});
beforeEach(() => navigate("workouts"));

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
  /** A trainer is paired (`core.rs:534`). */
  trainerPaired: boolean;
  /** The trainer does not answer within the connect timeout (`core.rs:542`). */
  connectTimesOut: boolean;
  /** `ride_load` calls currently in flight (the core's `loading` flag, `core.rs:677`). */
  loading: number;
  /** Hold the next `ride_load` answer until the returned function is called. */
  gate: Promise<void> | null;
}
/**
 * The core's workout commands behind `mockIPC`. `ride_load` refuses, in the core's order (crates/crank-core/src/core.rs):
 * `not_found` for an id outside the library (`Library::get`, library.rs:79); `invalid_input` with the issue's
 * message for a workout with an error-severity issue (`resolve_workout`, :618-628); `conflict` "a ride is in
 * progress" (:660); `conflict` "another ride is loading or loaded" while another load is in flight (:677-679);
 * `device_unavailable` "no trainer paired" (:534) and "trainer connect timed out" (:542). `ensure_start_allowed`
 * (:524) always passes today and is not modelled. A free ride (null id) skips the workout checks only. Tauri
 * itself rejects a call without its `workoutId` argument with a plain string.
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
        if (!b.trainerPaired)
          throw apiError("device_unavailable", "no trainer paired");
        if (b.connectTimesOut)
          throw apiError("device_unavailable", "trainer connect timed out");
        const done = () =>
          ({
            rideId: "ride-1",
            workoutId: id,
            hrLinked: false,
          }) satisfies RideLoaded;
        const gate = b.gate;
        if (!gate) return done();
        b.gate = null;
        b.loading++;
        return gate.finally(() => b.loading--).then(done);
      }
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
  // The library changed between the pick and the click: the core re-checks at load (core.rs:618-628).
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
