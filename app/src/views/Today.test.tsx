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
import { clearMocks } from "@tauri-apps/api/mocks";
import {
  afterAll,
  afterEach,
  beforeAll,
  beforeEach,
  expect,
  test,
  vi,
} from "vitest";
import type { PairedDevice, RideRecord } from "../bindings";
import { navigate, useRoute } from "../router";
import { setTimeZone, withTimeZone } from "../testTz";
import { apiError, RIDES } from "./historyBackend";
import { resetRideLoad, startRideLoad } from "./rideLoad";
import { RideLoadNotice } from "./RideLoadNotice";
import { lastSevenDays, Today } from "./Today";
import { install, type Backend } from "./todayBackend";
import { install as installWorkouts } from "./workoutsBackend";

// Dates render in the rider's local time: pin it, and the clock, so a day can be asserted literally.
const NOW = new Date("2026-11-02T08:00:00Z");
/** 2026-11-02T00:00Z in Unix seconds, and one day. */
const MIDNIGHT = 1_793_577_600;
const DAY = 86_400;
let restoreTimeZone: () => void;
beforeAll(() => {
  restoreTimeZone = setTimeZone("UTC");
});
afterAll(() => restoreTimeZone());
beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(NOW);
  resetRideLoad();
  navigate("today");
});
afterEach(() => {
  vi.useRealTimers();
  cleanup();
  clearMocks();
});

const neo: PairedDevice = {
  role: "trainer",
  id: "neo-1",
  name: "Tacx Neo 55624",
  serial: "55624",
  model: "Neo",
};
const hrm: PairedDevice = {
  role: "heart_rate",
  id: "hrm-1",
  name: "HRM-Dual",
  serial: null,
  model: null,
};
const ride = (over: Partial<RideRecord>): RideRecord => ({
  ...RIDES[0],
  ...over,
});

function Routed() {
  const route = useRoute();
  return (
    <>
      <p data-testid="route">
        {route.view}
        {route.sub ? `/${route.sub}` : ""}
      </p>
      {route.view === "today" && <Today />}
      <RideLoadNotice />
    </>
  );
}
const open = () => render(<Routed />);
const route = () => screen.getByTestId("route").textContent;
const panel = (name: string) => within(screen.getByRole("region", { name }));

/** The workouts backend answers the load (`ride_load`), the Today backend the rest, as one core would. */
const withLoad = (over: Partial<Backend> = {}) => {
  const w = installWorkouts();
  const internals = (
    window as unknown as {
      __TAURI_INTERNALS__: {
        invoke: (cmd: string, args: unknown) => Promise<unknown>;
      };
    }
  ).__TAURI_INTERNALS__;
  const others = internals.invoke.bind(internals);
  return { w, b: install({ others, ...over }) };
};

test("with nothing loaded, today's ride says nothing is planned and leads to Workouts", async () => {
  install();
  open();
  const ride = panel("Today’s ride");
  expect(ride.getByText("Nothing planned")).toBeTruthy();
  await userEvent.click(ride.getByRole("button", { name: "Pick a workout" }));
  expect(route()).toBe("workouts");
});

test("while a ride loads Start names what it is doing and cannot be pressed", async () => {
  const { w } = withLoad();
  let release = () => {};
  w.gate = new Promise<void>((r) => (release = r));
  open();
  const loading = startRideLoad("w-easy", "Easy spin");
  const start = await panel("Today’s ride").findByRole("button", {
    name: "Loading ride…",
  });
  expect((start as HTMLButtonElement).disabled).toBe(true);
  expect(start.getAttribute("aria-busy")).toBe("true");
  expect(panel("Today’s ride").getByText("Easy spin")).toBeTruthy();
  release();
  await act(() => loading);
  expect(
    panel("Today’s ride").getByRole("button", { name: "Start" }),
  ).toBeTruthy();
});

test("a loaded ride is today's ride: Start leads to its ready check, and the notice does not say it twice", async () => {
  const { w } = withLoad();
  let release = () => {};
  w.gate = new Promise<void>((r) => (release = r));
  open();
  const loading = startRideLoad("w-easy", "Easy spin");
  release();
  await act(() => loading);
  const ride = panel("Today’s ride");
  expect(ride.getByText("Easy spin")).toBeTruthy();
  // The key hint is drawn, outside the button's name (§18.4).
  const start = ride.getByRole("button", { name: "Start" });
  expect(start.querySelector("kbd")?.textContent).toBe("⏎");
  expect(screen.queryByText(/Ride loaded/)).toBeNull();
  // Elsewhere the notice still offers it.
  await act(() => navigate("settings"));
  expect(screen.getByText(/Ride loaded: Easy spin/)).toBeTruthy();
  await act(() => navigate("today"));
  await userEvent.click(
    panel("Today’s ride").getByRole("button", { name: "Start" }),
  );
  expect(route()).toBe("ride");
});

test("a refused load is the notice's, and today's ride is back to nothing planned", async () => {
  const { w } = withLoad();
  w.trainerPaired = false;
  open();
  await act(() => startRideLoad("w-easy", "Easy spin"));
  const alert = await screen.findByRole("alert");
  expect(alert.textContent).toMatch(/Could not load Easy spin/);
  expect(panel("Today’s ride").getByText("Nothing planned")).toBeTruthy();
});

test("devices list who is paired for each role", async () => {
  install({ paired: { trainer: neo, heart_rate: hrm } });
  open();
  const list = await panel("Devices").findByRole("list", {
    name: "Paired devices",
  });
  const rows = within(list).getAllByRole("listitem");
  expect(rows.map((r) => r.textContent)).toEqual([
    "TrainerTacx Neo 55624✓ paired",
    "Heart-rate strapHRM-Dual✓ paired",
  ]);
  await userEvent.click(
    panel("Devices").getByRole("button", { name: "Devices" }),
  );
  expect(route()).toBe("settings/devices");
});

test("without a paired trainer the devices say a ride needs one and lead to pairing", async () => {
  install({ paired: { heart_rate: hrm } });
  open();
  const devices = panel("Devices");
  expect(
    await devices.findByText(/A ride needs a paired trainer/),
  ).toBeTruthy();
  const rows = devices.getAllByRole("listitem");
  expect(rows[0].textContent).toBe("Trainer—⚠ not paired");
  expect(rows[0].dataset.state).toBe("missing");
  await userEvent.click(
    devices.getByRole("button", { name: "Pair a trainer" }),
  );
  expect(route()).toBe("settings/devices");
});

test("while a ride is loaded the devices show its links, read again once the load settles", async () => {
  const { w, b } = withLoad({ paired: { trainer: neo, heart_rate: hrm } });
  let release = () => {};
  w.gate = new Promise<void>((r) => (release = r));
  open();
  await panel("Devices").findAllByText(/paired/);
  const reads = () => b.calls.filter((c) => c.cmd === "device_links").length;
  const before = reads();
  const loading = startRideLoad("w-easy", "Easy spin");
  b.links = { trainer: "ready", heart_rate: "connecting" };
  release();
  await act(() => loading);
  await waitFor(() => expect(reads()).toBeGreaterThan(before));
  const rows = panel("Devices").getAllByRole("listitem");
  await waitFor(() =>
    expect(rows.map((r) => r.textContent)).toEqual([
      "TrainerTacx Neo 55624✓ connected",
      "Heart-rate strapHRM-Dual↻ connecting",
    ]),
  );
});

test("a failed device read is a notice with Retry", async () => {
  const b = install({
    fail: { device_links: apiError("storage", "database is locked") },
  });
  open();
  const devices = panel("Devices");
  const alert = await devices.findByRole("alert");
  expect(alert.querySelector("p")?.textContent).toBe("⚠ database is locked");
  delete b.fail.device_links;
  await userEvent.click(within(alert).getByRole("button", { name: "Retry" }));
  expect(await devices.findByRole("list")).toBeTruthy();
  expect(devices.queryByRole("alert")).toBeNull();
});

test("the last ride is the newest one, with its figures and a Review named like History's", async () => {
  install();
  open();
  const last = panel("Last ride");
  expect(await last.findByText("Sweet spot 3x12")).toBeTruthy();
  expect(last.getByText("50:00")).toBeTruthy();
  expect(last.getByText("540 kJ")).toBeTruthy();
  expect(last.getByText("Finished")).toBeTruthy();
  const review = last.getByRole("button", {
    name: /^Review Sweet spot 3x12, .*2025.*, Finished, 50:00$/,
  });
  await userEvent.click(review);
  expect(route()).toBe("history/ride-1");
});

test("with no rides the last ride says where rides go, and the week is all rest", async () => {
  install({ rides: [] });
  open();
  expect(await panel("Last ride").findByText(/No rides yet/)).toBeTruthy();
  const days = panel("Last 7 days").getAllByRole("listitem");
  expect(days).toHaveLength(7);
  expect(days.map((d) => d.dataset.state)).toEqual([
    ...Array(6).fill("rest"),
    "today",
  ]);
});

test("a failed ride read is one notice with Retry, and the week says it could not be read", async () => {
  const b = install({
    fail: { rides_list: apiError("storage", "disk I/O error") },
  });
  open();
  const alert = await panel("Last ride").findByRole("alert");
  expect(alert.querySelector("p")?.textContent).toBe("⚠ disk I/O error");
  expect(
    panel("Last 7 days").getByText("Your rides could not be read."),
  ).toBeTruthy();
  expect(screen.getAllByRole("alert")).toHaveLength(1);
  delete b.fail.rides_list;
  await userEvent.click(within(alert).getByRole("button", { name: "Retry" }));
  expect(await panel("Last ride").findByText("Sweet spot 3x12")).toBeTruthy();
});

test("while the reads are in flight each panel says it is loading", async () => {
  install({
    hold: {
      rides_list: new Promise(() => {}),
      device_links: new Promise(() => {}),
    },
  });
  open();
  for (const name of ["Devices", "Last 7 days", "Last ride"])
    expect(panel(name).getByText("Loading…")).toBeTruthy();
});

test("the week reads each day's cells: its name, date, minutes ridden and state", async () => {
  install({
    rides: [
      ride({ id: "a", startedAt: MIDNIGHT + 6 * 3600 }),
      ride({ id: "b", startedAt: MIDNIGHT - 2 * DAY + 3600 }),
      ride({
        id: "c",
        startedAt: MIDNIGHT - 2 * DAY + 7200,
        summary: { ...RIDES[0].summary!, durationS: 1200 },
      }),
      ride({
        id: "d",
        startedAt: MIDNIGHT - 4 * DAY,
        outcome: "aborted",
        summary: null,
      }),
    ],
  });
  open();
  const week = await panel("Last 7 days").findByRole("list", {
    name: "Last 7 days",
  });
  const days = within(week).getAllByRole("listitem");
  expect(days.map((d) => d.textContent)).toEqual([
    "TueOct 27—rest",
    "WedOct 28—rest",
    "ThuOct 29—1 ride",
    "FriOct 30—rest",
    "SatOct 3170 min2 rides",
    "SunNov 1—rest",
    "TodayNov 250 min1 ride",
  ]);
  expect(days[6].getAttribute("aria-current")).toBe("date");
});

test("the seven days are calendar days across a daylight-saving change", async () => {
  await withTimeZone("Europe/Zurich", () => {
    // 2026-10-25 00:30 in Zurich (22:30 UTC the day before); the clocks go back at 03:00 that morning.
    const r = ride({ id: "dst", startedAt: 1_792_881_000 });
    const days = lastSevenDays([r], Date.parse("2026-10-28T10:00:00Z"));
    expect(days.map((d) => new Date(d.start * 1000).getDate())).toEqual([
      22, 23, 24, 25, 26, 27, 28,
    ]);
    expect(days.map((d) => d.rides)).toEqual([0, 0, 0, 1, 0, 0, 0]);
  });
});

test("a ride outside the seven days or without a start is not counted", () => {
  const days = lastSevenDays(
    [
      ride({ id: "old", startedAt: MIDNIGHT - 7 * DAY }),
      ride({ id: "none", startedAt: null }),
    ],
    NOW.getTime(),
  );
  expect(days.every((d) => d.rides === 0)).toBe(true);
});
