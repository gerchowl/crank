// @vitest-environment jsdom
import { act, cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import {
  afterAll,
  afterEach,
  beforeAll,
  beforeEach,
  expect,
  test,
} from "vitest";
import { commands } from "../bindings";
import { apiError, install } from "./historyBackend";
import { markForcedEnd, resetForcedEnds } from "../ride/forcedEnds";
import { navigate, useRoute } from "../router";
import { setTimeZone } from "../testTz";
import { History, startedText, work } from "./History";
import { axis, extent, ticks } from "./RideTraces";
import { beginRide, releaseRide, type Point } from "../ride/session";

// The start time renders in the rider's local time: pin it, so a date can be asserted literally.
let restoreTimeZone: () => void;
beforeAll(() => {
  restoreTimeZone = setTimeZone("UTC");
});
afterAll(() => restoreTimeZone());
afterEach(() => {
  resetForcedEnds();
  cleanup();
  clearMocks();
});
beforeEach(() => navigate("history", "ride-1"));

function Routed() {
  const route = useRoute();
  return (
    <>
      <p data-testid="route">{route.view}</p>
      {route.view === "history" && <History sub={route.sub} />}
    </>
  );
}
const open = () => render(<Routed />);

test("a summary renders the core's values", async () => {
  const b = install();
  open();
  expect(
    await screen.findByRole("heading", { name: "Sweet spot 3x12" }),
  ).toBeTruthy();
  expect(b.calls.find((c) => c.cmd === "ride_summary")?.args).toEqual({
    id: "ride-1",
  });
  const s = within(screen.getByRole("region", { name: "Ride summary" }));
  expect(s.getByText("Finished")).toBeTruthy();
  expect(s.queryByRole("note")).toBeNull();
  // 1_760_000_000 s is 2025-10-09 08:53:20 UTC (read as milliseconds it would be 1970)
  const started = s.getByText(/2025/).textContent ?? "";
  expect(started).toMatch(/0?8:53/);
  expect(startedText(1_760_000_000)).toBe(started);
  const value = (term: string) =>
    s.getByText(term).nextElementSibling?.textContent;
  expect(value("Duration")).toBe("50:00");
  expect(value("Average power")).toBe("187 W");
  expect(value("Normalized power")).toBe("202 W");
  expect(value("Average heart rate")).toBe("142 bpm");
  expect(value("Work")).toBe("540 kJ");
  // planned and ridden per block; a block with no recorded power shows a dash
  const rows = within(s.getByRole("table", { name: "Blocks" })).getAllByRole(
    "row",
  );
  expect(rows[1]!.textContent).toContain("10:00 · 150 W");
  expect(rows[1]!.textContent).toContain("10:00 · 148 W");
  expect(within(rows[2]!).getAllByRole("cell")[1]!.textContent).toBe(
    "11:40 · —",
  );
});

test("a short ride's work keeps a decimal", async () => {
  install();
  navigate("history", "ride-3");
  open();
  const s = within(await screen.findByRole("region", { name: "Ride summary" }));
  expect(s.getByText("Work").nextElementSibling?.textContent).toBe("0.6 kJ");
});

test("an aborted ride that was not ended by force shows no forced note", async () => {
  install();
  navigate("history", "ride-2");
  open();
  const s = within(await screen.findByRole("region", { name: "Ride summary" }));
  expect(s.getByText("Aborted")).toBeTruthy();
  expect(s.queryByRole("note")).toBeNull();
});

test("a ride the core ended by force shows the note, whatever its recorded outcome", async () => {
  install();
  markForcedEnd("ride-1");
  navigate("history", "ride-1");
  open();
  const s = within(await screen.findByRole("region", { name: "Ride summary" }));
  expect(s.getByText("Finished")).toBeTruthy();
  expect(s.getByRole("note").textContent).toBe(
    "Ride ended without the engine confirming; data may be incomplete.",
  );
});

test("a forced end the core answers after the summary is showing adds the note (#372)", async () => {
  install();
  navigate("history", "ride-1");
  open();
  const s = within(await screen.findByRole("region", { name: "Ride summary" }));
  // `findBy` resolves on the DOM change, before React has run the passive effects of that commit, and the summary's
  // `useForcedEnd` subscribes in one of them: let them run, so the mark below reaches a live subscriber and its
  // re-render is flushed by the `act` that marks (without this, 7 of 10 runs of 300 repeats failed)
  await act(async () => {});
  expect(s.queryByRole("note")).toBeNull();
  // the `finished` frame routed here before the `ride_end` answer; the answer marks the ride forced
  act(() => markForcedEnd("ride-1"));
  expect(s.getByRole("note").textContent).toBe(
    "Ride ended without the engine confirming; data may be incomplete.",
  );
});

test("an aborted ride has no summary to show", async () => {
  install();
  navigate("history", "ride-2");
  open();
  const s = within(await screen.findByRole("region", { name: "Ride summary" }));
  expect(s.getByText("Aborted")).toBeTruthy();
  expect(s.getByText("No summary was recorded for this ride.")).toBeTruthy();
});

test("a ride that is not stored says so and offers the way back", async () => {
  install();
  navigate("history", "ride-404");
  open();
  expect((await screen.findByRole("alert")).textContent).toBe(
    "⚠ This ride isn’t in your history.",
  );
  expect(screen.queryByRole("region", { name: "Ride summary" })).toBeNull();
  await userEvent.click(
    screen.getByRole("button", { name: "Back to Workouts" }),
  );
  expect(screen.getByTestId("route").textContent).toBe("workouts");
});

test("a store failure is an error with Retry, which asks again", async () => {
  const b = install({
    fail: { ride_summary: apiError("storage", "database is locked") },
  });
  open();
  expect((await screen.findByRole("alert")).textContent).toContain(
    "database is locked",
  );
  delete b.fail.ride_summary;
  await userEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(
    await screen.findByRole("heading", { name: "Sweet spot 3x12" }),
  ).toBeTruthy();
  expect(b.calls.filter((c) => c.cmd === "ride_summary")).toHaveLength(2);
});

test("the way back leads to Workouts, Today or the ride list", async () => {
  install();
  open();
  await screen.findByRole("region", { name: "Ride summary" });
  await userEvent.click(screen.getByRole("button", { name: "Today" }));
  expect(screen.getByTestId("route").textContent).toBe("today");
  navigate("history", "ride-1");
  await userEvent.click(
    await screen.findByRole("button", { name: "All rides" }),
  );
  expect(screen.getByTestId("route").textContent).toBe("history");
  expect(await screen.findByRole("table", { name: "Rides" })).toBeTruthy();
});

test("the list links each ride to its summary", async () => {
  install();
  navigate("history");
  open();
  const list = within(await screen.findByRole("table", { name: "Rides" }));
  expect(list.getAllByRole("button")).toHaveLength(3);
  await userEvent.click(list.getByRole("button", { name: /Free ride/ }));
  expect(
    await screen.findByText("No summary was recorded for this ride."),
  ).toBeTruthy();
});

test("the list says when there are no rides, and shows a store failure", async () => {
  mockIPC((cmd) => {
    if (cmd === "rides_list") return [];
    throw `command ${cmd} not found`;
  });
  navigate("history");
  open();
  expect(await screen.findByText("No rides yet.")).toBeTruthy();
  cleanup();
  install({ fail: { rides_list: apiError("storage", "disk I/O error") } });
  open();
  expect((await screen.findByRole("alert")).textContent).toContain(
    "disk I/O error",
  );
});

test("Loading shows while the summary and the list are being fetched", async () => {
  let release!: () => void;
  const gate = new Promise<void>((r) => (release = r));
  install({ hold: { ride_summary: gate, rides_list: gate } });
  open();
  expect(await screen.findByText("Loading…")).toBeTruthy();
  cleanup();
  navigate("history");
  open();
  expect(await screen.findByText("Loading…")).toBeTruthy();
  act(() => release());
  expect(await screen.findByRole("table", { name: "Rides" })).toBeTruthy();
  expect(screen.queryByText("Loading…")).toBeNull();
});

test("the list's Retry asks again", async () => {
  const b = install({
    fail: { rides_list: apiError("storage", "disk I/O error") },
  });
  navigate("history");
  open();
  await screen.findByRole("alert");
  delete b.fail.rides_list;
  await userEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(await screen.findByRole("table", { name: "Rides" })).toBeTruthy();
  expect(b.calls.filter((c) => c.cmd === "rides_list")).toHaveLength(2);
});

test("changing the ride shows neither the old ride nor the old failure", async () => {
  let release!: () => void;
  const gate = new Promise<void>((r) => (release = r));
  install({ hold: { "ride_summary:ride-2": gate } });
  navigate("history", "ride-404");
  open();
  await screen.findByRole("alert");
  // another ride is asked for: the not-found alert of the first is gone while it loads
  act(() => navigate("history", "ride-2"));
  expect(await screen.findByText("Loading…")).toBeTruthy();
  expect(screen.queryByRole("alert")).toBeNull();
  act(() => release());
  expect(await screen.findByText("Aborted")).toBeTruthy();
});

test("a stale answer for the previous ride is not shown for the next", async () => {
  let release!: () => void;
  const gate = new Promise<void>((r) => (release = r));
  install({ hold: { "ride_summary:ride-2": gate } });
  open();
  await screen.findByRole("heading", { name: "Sweet spot 3x12" });
  act(() => navigate("history", "ride-2"));
  expect(await screen.findByText("Loading…")).toBeTruthy();
  // ride-1's summary is still in the hook's state: it must not stand in for ride-2
  expect(screen.queryByRole("region", { name: "Ride summary" })).toBeNull();
  expect(screen.queryByText("Sweet spot 3x12")).toBeNull();
  act(() => release());
  expect(await screen.findByText("Aborted")).toBeTruthy();
});

test("work has one rule at the 10 kJ boundary: the format follows the rounded value", () => {
  expect(work(9.94)).toBe("9.9 kJ");
  expect(work(9.95)).toBe("10 kJ");
  expect(work(9.99)).toBe("10 kJ");
  expect(work(10.0)).toBe("10 kJ");
  expect(work(10.05)).toBe("10 kJ");
  expect(work(10.4)).toBe("10 kJ");
  expect(work(-9.95)).toBe("-10 kJ");
  expect(work(0.6)).toBe("0.6 kJ");
  expect(work(null)).toBe("—");
});

test("work never prints a negative zero", () => {
  expect(work(-0)).toBe("0.0 kJ");
  expect(work(-0.04)).toBe("0.0 kJ");
  expect(work(-0.0001)).toBe("0.0 kJ");
  expect(work(-0.06)).toBe("-0.1 kJ");
});

test("the list hides its old error while Retry is in flight, then shows the new outcome", async () => {
  let release!: () => void;
  const gate = new Promise<void>((r) => (release = r));
  let calls = 0;
  mockIPC((cmd) => {
    if (cmd !== "rides_list") throw `command ${cmd} not found`;
    calls += 1;
    if (calls === 1) throw apiError("storage", "disk I/O error");
    return gate.then(() => {
      throw apiError("storage", "disk is full");
    });
  });
  navigate("history");
  open();
  expect((await screen.findByRole("alert")).textContent).toContain(
    "disk I/O error",
  );
  await userEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(screen.queryByRole("alert")).toBeNull();
  expect(screen.queryByText(/disk I\/O error/)).toBeNull();
  act(() => release());
  expect((await screen.findByRole("alert")).textContent).toContain(
    "disk is full",
  );
});

/** The window's session of `rideId`, with `seconds` of samples at `hz` per series (power, heart rate, cadence). */
function rode(rideId: string, seconds: number, hz = 4) {
  const session = beginRide({ rideId, workoutId: null, hrLinked: true });
  const of = (v: (t: number) => number): Point[] =>
    Array.from({ length: seconds * hz }, (_, i) => {
      const t = i / hz;
      return { t, workoutS: t, v: v(t) };
    });
  // The series as `record` leaves them (a fresh array each), set at once: appending 43 200 samples one by one is
  // #557's O(n²) and not what this test is about.
  session.series = {
    power: of((t) => 200 + (t % 60)),
    hr: of((t) => 120 + (t % 30)),
    cadence: of(() => 90),
  };
  return session;
}

test("Review draws the traces of the ride this window rode, one vertex per second", async () => {
  install();
  const session = rode("ride-3", 300);
  navigate("history", "ride-3");
  open();
  const s = within(await screen.findByRole("region", { name: "Ride summary" }));
  for (const name of ["Power", "Heart rate", "Cadence"])
    expect(
      s.getByRole("img", { name: new RegExp(`^${name} over the ride`) }),
    ).toBeTruthy();
  const power = s.getAllByTestId("review-power");
  expect(power).toHaveLength(1);
  expect(power[0]!.getAttribute("points")!.split(" ")).toHaveLength(300);
  // heart rate is on the 80–200 bpm axis of DESIGN §5.5, whatever its samples
  expect(
    s.getByRole("img", { name: /^Heart rate/ }).getAttribute("aria-label"),
  ).toBe("Heart rate over the ride, 80 to 200 bpm");
  releaseRide(session);
});

test("a ride the window did not ride shows its figures and no traces", async () => {
  install();
  const session = rode("ride-3", 300);
  navigate("history", "ride-1");
  open();
  const s = within(await screen.findByRole("region", { name: "Ride summary" }));
  expect(s.queryByRole("img")).toBeNull();
  expect(
    s.getByText(/Traces show for the ride you have just ridden/),
  ).toBeTruthy();
  expect(s.getByText("Duration").nextElementSibling?.textContent).toBe("50:00");
  releaseRide(session);
});

test("a 3 h ride at 4 Hz renders its traces whole", async () => {
  install();
  const session = rode("ride-1", 3 * 3600);
  navigate("history", "ride-1");
  open();
  const s = within(await screen.findByRole("region", { name: "Ride summary" }));
  for (const series of ["power", "hr", "cadence"]) {
    const lines = s.getAllByTestId(`review-${series}`);
    expect(lines).toHaveLength(1);
    expect(lines[0]!.getAttribute("points")!.split(" ")).toHaveLength(10_800);
  }
  // the axis ends at the last sample (3:00:00 less one 4 Hz step, shown rounded), past the summary's 50:00
  expect(s.getByText("3:00:00")).toBeTruthy();
  releaseRide(session);
});

test("a trace's range is found without spreading the samples into Math.max", () => {
  // Far past any engine's argument limit: `Math.max(...values)` throws a RangeError here.
  const points: Point[] = Array.from({ length: 500_000 }, (_, i) => ({
    t: i / 4,
    workoutS: i / 4,
    v: i === 123_456 ? 612 : 150 + (i % 50),
  }));
  expect(() => Math.max(...points.map((p) => p.v))).toThrow(RangeError);
  expect(extent(points)).toEqual([150, 612]);
  expect(axis("power", points)).toEqual([0, 673]);
  expect(axis("hr", points)).toEqual([80, 200]);
  expect(axis("cadence", points)).toEqual([140, 622]);
  expect(extent([])).toBeNull();
  expect(axis("power", [])).toEqual([0, 110]);
});

test("time ticks are at most six and keep clear of the axis end", () => {
  expect(ticks(300)).toEqual([60, 120, 180, 240]);
  expect(ticks(3000)).toEqual([600, 1200, 1800, 2400]);
  expect(ticks(10_800)).toEqual([1800, 3600, 5400, 7200, 9000]);
  for (const end of [61, 300, 3000, 10_800, 43_200])
    for (const t of ticks(end)) expect(t).toBeLessThan(end * 0.85);
});

test("the list is newest first, and marks the ride last opened on the way back", async () => {
  install();
  navigate("history", "ride-3");
  open();
  await screen.findByRole("region", { name: "Ride summary" });
  await userEvent.click(screen.getByRole("button", { name: "All rides" }));
  const list = within(await screen.findByRole("table", { name: "Rides" }));
  // The rows after the column heads'.
  const rows = list.getAllByRole("row").slice(1);
  // started 1_760_000_000, 1_759_000_000, 1_758_000_000
  expect(rows.map((r) => within(r).getByRole("rowheader").textContent)).toEqual(
    [
      expect.stringContaining("Sweet spot 3x12"),
      expect.stringContaining("Free ride"),
      expect.stringContaining("Spin"),
    ],
  );
  expect(rows.map((r) => r.getAttribute("aria-current"))).toEqual([
    null,
    null,
    "true",
  ]);
  expect(within(rows[2]!).getByText("Last opened")).toBeTruthy();
});

test("a store failure of the list fails the summary too, as the core's ride_summary reads the list", async () => {
  install({ fail: { rides_list: apiError("storage", "disk I/O error") } });
  navigate("history", "ride-1");
  open();
  expect((await screen.findByRole("alert")).textContent).toContain(
    "disk I/O error",
  );
});

test("a ride's cells are named by their column heads, and its Review button by the ride", async () => {
  install();
  navigate("history");
  open();
  const table = within(await screen.findByRole("table", { name: "Rides" }));
  expect(table.getAllByRole("columnheader").map((h) => h.textContent)).toEqual([
    "Started",
    "Ride",
    "Duration",
    "Work",
    "Outcome",
    "Review",
  ]);
  const [, first] = table.getAllByRole("row");
  const row = within(first!);
  expect(row.getByRole("rowheader").textContent).toBe("Sweet spot 3x12");
  expect(row.getAllByRole("cell").map((c) => c.textContent)).toEqual([
    startedText(1_760_000_000),
    "50:00",
    "540 kJ",
    "Finished",
    "Review",
  ]);
  // Each name in full: a screen reader hears the ride, its start, its outcome and its duration on the button.
  for (const name of [
    `Review Sweet spot 3x12, ${startedText(1_760_000_000)}, Finished, 50:00`,
    `Review Free ride, ${startedText(1_759_000_000)}, Aborted, —`,
    `Review Spin, ${startedText(1_758_000_000)}, Finished, 5:00`,
  ])
    expect(table.getByRole("button", { name })).toBeTruthy();
  expect(table.getAllByRole("button")).toHaveLength(3);
});

test("the ride last opened is History's: a new History, or one left for another view, marks none", async () => {
  install();
  navigate("history", "ride-3");
  open();
  await screen.findByRole("region", { name: "Ride summary" });
  cleanup();
  navigate("history");
  open();
  const table = within(await screen.findByRole("table", { name: "Rides" }));
  expect(table.queryByText("Last opened")).toBeNull();
  await userEvent.click(table.getByRole("button", { name: /^Review Spin/ }));
  await userEvent.click(await screen.findByRole("button", { name: "Today" }));
  navigate("history");
  await screen.findByRole("table", { name: "Rides" });
  expect(screen.queryByText("Last opened")).toBeNull();
});

test("an unknown id is refused as the core words it, the id in Rust's debug quoting", async () => {
  const b = install();
  await expect(commands.rideSummary('a"b\\c\n')).resolves.toEqual({
    status: "error",
    error: { code: "not_found", message: 'no ride "a\\"b\\\\c\\n"' },
  });
  expect(b.calls).toHaveLength(1);
});
