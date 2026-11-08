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
import { invoke } from "@tauri-apps/api/core";
import { clearMocks } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach, expect, test } from "vitest";
import { navigate, useRoute } from "../router";
import { currentSession } from "../ride/session";
import { Workouts } from "./Workouts";
import { RideLoadNotice } from "./RideLoadNotice";
import { resetRideLoad, startRideLoad } from "./rideLoad";
import { block } from "../ride/rideBackend";
import {
  ERROR_ISSUE,
  WARNING,
  apiError,
  compiled,
  install,
  type Backend,
} from "./workoutsBackend";

afterEach(() => {
  cleanup();
  clearMocks();
});
beforeEach(() => {
  resetRideLoad();
  navigate("workouts");
});

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
    await screen.findByRole("option", { name: /Sweet spot 3x12/ }),
  ).toBeTruthy();
  expect(screen.getByRole("option", { name: /Easy spin/ })).toBeTruthy();
  // list rows carry the summary's duration and stress
  const row = screen.getByRole("option", { name: /Sweet spot 3x12/ });
  expect(row.textContent).toMatch(/50:00/);
  expect(row.textContent).toMatch(/TSS 52/);
});

test("picking a workout shows the compiled summary from the core", async () => {
  const b = install();
  open();
  await userEvent.click(
    await screen.findByRole("option", { name: /Sweet spot 3x12/ }),
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
    await screen.findByRole("option", { name: /Sweet spot 3x12/ }),
  );
  const s = within(await summary());
  const issue = await s.findByText(/W′ reserve drops below 15 percent/);
  expect(issue.closest("li")!.textContent).toMatch(/Warning/);
});

test("Ride this loads the picked workout and routes to the ride", async () => {
  const b = install();
  open();
  await userEvent.click(
    await screen.findByRole("option", { name: /Easy spin/ }),
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
    await screen.findByRole("option", { name: /Sweet spot 3x12/ }),
  ).toBeTruthy();
  expect(screen.queryByRole("alert")).toBeNull();
});

test("a workout the core cannot give shows the error instead of a summary", async () => {
  // The library changed between the list and the pick (the store-backed library of #110): w-ss is gone.
  const b = install();
  open();
  const row = await screen.findByRole("option", { name: /Sweet spot 3x12/ });
  b.summaries = b.summaries.filter((w) => w.id !== "w-ss");
  await userEvent.click(row);
  expect((await screen.findByRole("alert")).textContent).toMatch(
    /no workout "w-ss"/,
  );
  expect(screen.queryByRole("region", { name: "Compiled summary" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Ride this" })).toBeNull();
});

test("a refused ride-load shows the core's reason and stays on the page", async () => {
  install({ rideActive: true });
  open();
  await userEvent.click(
    await screen.findByRole("option", { name: /Sweet spot 3x12/ }),
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
  // The library changed between the pick and the click: the core re-checks at load (core.rs `resolve_workout` (~1349)).
  const b = install();
  open();
  await userEvent.click(
    await screen.findByRole("option", { name: /Sweet spot 3x12/ }),
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
    await screen.findByRole("option", { name: /Easy spin/ }),
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
    await screen.findByRole("option", { name: /Easy spin/ }),
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
    await screen.findByRole("option", { name: /Sweet spot 3x12/ }),
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
    await screen.findByRole("option", { name: /Sweet spot 3x12/ }),
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
    await screen.findByRole("option", { name: /Easy spin/ }),
  );
  await summary();
  await userEvent.click(
    await screen.findByRole("button", { name: "Ride this" }),
  );
  await screen.findByRole("alert");
  await userEvent.click(
    screen.getByRole("option", { name: /Sweet spot 3x12/ }),
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
    await screen.findByRole("option", { name: /Easy spin/ }),
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

// The library is a listbox (DESIGN §5.7 in direction A, #528): a picked row is selected, a locked one disabled.
const pressed = (name: RegExp) =>
  screen.getByRole("option", { name }).getAttribute("aria-selected");
const locked = (row: HTMLElement) =>
  row.getAttribute("aria-disabled") === "true";
const startHeldLoad = async (b: Backend, first: RegExp) => {
  let release = () => {};
  b.gate = new Promise<void>((resolve) => (release = resolve));
  open();
  await userEvent.click(await screen.findByRole("option", { name: first }));
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
    expect(locked(screen.getByRole("option", { name }))).toBe(true);
  await userEvent.click(
    screen.getByRole("option", { name: /Sweet spot 3x12/ }),
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
    screen.getByRole("option", { name: /Sweet spot 3x12/ }),
  );
  expect(pressed(/Easy spin/)).toBe("true");

  release();
  const alert = await screen.findByRole("alert");
  expect(alert.textContent).toMatch(/connect failed: out of range/);
  // still the first workout's page: its name is the heading the refusal sits under
  expect(screen.getByRole("heading", { name: "Easy spin" })).toBeTruthy();
  expect(pressed(/Easy spin/)).toBe("true");
  expect(screen.queryByText("Loading ride…")).toBeNull();
  expect(locked(screen.getByRole("option", { name: /Sweet spot 3x12/ }))).toBe(
    false,
  );
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
  [
    "a quit during the load",
    { build: apiError("conflict", "the application is quitting") },
    /the application is quitting/,
  ],
  [
    "the link keeper",
    { connect: apiError("internal", "the link keeper is gone") },
    /the link keeper is gone/,
  ],
];
for (const [where, stage, message] of REFUSALS)
  test(`a refusal from ${where} shows the core's message and stays on the page`, async () => {
    install({ stage });
    open();
    await userEvent.click(
      await screen.findByRole("option", { name: /Easy spin/ }),
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
    expect(locked(await screen.findByRole("option", { name }))).toBe(true);
  await userEvent.click(
    screen.getByRole("option", { name: /Sweet spot 3x12/ }),
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
  expect(locked(await screen.findByRole("option", { name: /Easy spin/ }))).toBe(
    true,
  );
  b.coreLoading = false;
  await waitFor(() => expect(screen.queryByText("Loading ride…")).toBeNull());
  expect(locked(screen.getByRole("option", { name: /Easy spin/ }))).toBe(false);
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
    locked(await screen.findByRole("option", { name: /Sweet spot 3x12/ })),
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
    await screen.findByRole("option", { name: /Sweet spot 3x12/ }),
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

// ---- #528: direction A, the library as a listbox and the picked workout's panel ----

test("the library is one tab stop, and Up/Down/Home/End walk it and pick as they go", async () => {
  const b = install();
  open();
  const ss = await screen.findByRole("option", { name: /Sweet spot 3x12/ });
  const easy = screen.getByRole("option", { name: /Easy spin/ });
  // nothing picked: the first row is the stop
  expect(ss.tabIndex).toBe(0);
  expect(easy.tabIndex).toBe(-1);
  ss.focus();
  await userEvent.keyboard("{ArrowDown}");
  expect(document.activeElement).toBe(easy);
  expect(pressed(/Easy spin/)).toBe("true");
  expect(within(await summary()).getByText("0.78")).toBeTruthy();
  expect(easy.tabIndex).toBe(0);
  expect(ss.tabIndex).toBe(-1);
  await userEvent.keyboard("{ArrowDown}"); // the last row stays
  expect(document.activeElement).toBe(easy);
  await userEvent.keyboard("{Home}");
  expect(document.activeElement).toBe(ss);
  expect(pressed(/Sweet spot 3x12/)).toBe("true");
  await userEvent.keyboard("{End}");
  await userEvent.keyboard("{ArrowUp}");
  expect(pressed(/Sweet spot 3x12/)).toBe("true");
  expect(
    b.calls.filter((c) => c.cmd === "workout_get").map((c) => c.args.id),
  ).toEqual(["w-easy", "w-ss", "w-easy", "w-ss"]);
});

test("while a load is in flight the keys move nothing and pick nothing", async () => {
  const b = install();
  const release = await startHeldLoad(b, /Easy spin/);
  const easy = screen.getByRole("option", { name: /Easy spin/ });
  easy.focus();
  await userEvent.keyboard("{ArrowUp}{Home}{Enter}");
  expect(document.activeElement).toBe(easy);
  expect(pressed(/Easy spin/)).toBe("true");
  expect(b.calls.filter((c) => c.cmd === "workout_get")).toHaveLength(1);
  // Ride this names its pending state while its own load is in flight (DESIGN §5.9 "a pending one names what it is
  // doing"); no other line says it, so the table above does not move.
  const ride = screen.getByRole("button", { name: "Loading ride…" });
  expect(screen.getByText("Loading ride…")).toBe(ride);
  expect(screen.queryByRole("button", { name: "Ride this" })).toBeNull();
  expect(ride.getAttribute("aria-busy")).toBe("true");
  expect((ride as HTMLButtonElement).disabled).toBe(true);
  release();
  await waitFor(() =>
    expect(screen.getByTestId("route").textContent).toBe("ride"),
  );
});

test("an empty library says so and offers no rows", async () => {
  install({ summaries: [] });
  open();
  expect(await screen.findByText(/No workouts yet/)).toBeTruthy();
  expect(screen.queryByRole("listbox")).toBeNull();
});

test("the picked workout shows its profile and every block with its length and watts", async () => {
  install({
    compiled: {
      "w-ss": compiled({
        blocks: [
          block(0, 600, 110, 175, "warmup"),
          block(600, 720, 225, 225, "work"),
          block(1320, 300, 140, 100, "cooldown"),
        ],
      }),
    },
  });
  open();
  await userEvent.click(
    await screen.findByRole("option", { name: /Sweet spot 3x12/ }),
  );
  await summary();
  expect(
    screen.getByRole("img", { name: "Profile: 3 blocks over 50:00" }),
  ).toBeTruthy();
  const rows = within(
    screen.getByRole("region", { name: "Blocks" }),
  ).getAllByRole("listitem");
  expect(rows.map((r) => r.textContent)).toEqual([
    "1Warm-up10:00110–175 W",
    "2Work12:00225 W",
    "3Cool-down5:00140–100 W",
  ]);
});

// ---- #528 round 2: the mock refuses what the core refuses ----

test("the mock refuses a load while another window's load is in flight, as the core's one loading flag does", async () => {
  // core.rs `Core::ride_load` (~1679): `loading.swap(true)` is one flag for the whole core.
  install({ coreLoading: true });
  await expect(invoke("ride_load", { workoutId: "w-easy" })).rejects.toEqual({
    code: "conflict",
    message: "another ride is loading or loaded",
  });
});

test("the mock answers every command from one library: an id not in it is not_found", async () => {
  // library.rs `Library::get` (~75) serves `workout_get` and `ride_load` alike; `{id:?}` quotes the id.
  const b = install({ summaries: [] });
  expect(Object.keys(b.compiled)).toContain("w-ss");
  const notFound = { code: "not_found", message: 'no workout "w-ss"' };
  await expect(invoke("workout_get", { id: "w-ss" })).rejects.toEqual(notFound);
  await expect(invoke("ride_load", { workoutId: "w-ss" })).rejects.toEqual(
    notFound,
  );
});
