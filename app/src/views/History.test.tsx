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
import type { RideRecord } from "../bindings";
import { markForcedEnd, resetForcedEnds } from "../ride/forcedEnds";
import { navigate, useRoute } from "../router";
import { setTimeZone } from "../testTz";
import { History, startedText, work } from "./History";

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

const apiError = (code: string, message: string) => ({ code, message });

const RIDES: RideRecord[] = [
  {
    id: "ride-1",
    startedAt: 1_760_000_000,
    endedAt: 1_760_003_100,
    outcome: "finished",
    workoutId: "w-ss",
    workoutName: "Sweet spot 3x12",
    summary: {
      durationS: 3000,
      avgPowerW: 187.4,
      npW: 201.6,
      avgHrBpm: 142.2,
      kj: 540,
      blocks: [
        { index: 0, plannedS: 600, plannedW: 150, actualS: 600, actualW: 148 },
        { index: 1, plannedS: 720, plannedW: 230, actualS: 700, actualW: null },
      ],
    },
  },
  {
    id: "ride-3",
    startedAt: 1_758_000_000,
    endedAt: 1_758_000_300,
    outcome: "finished",
    workoutId: null,
    workoutName: "Spin",
    summary: {
      durationS: 300,
      avgPowerW: 2,
      npW: 2,
      avgHrBpm: null,
      kj: 0.6,
      blocks: [],
    },
  },
  {
    id: "ride-2",
    startedAt: 1_759_000_000,
    endedAt: 1_759_000_900,
    outcome: "aborted",
    workoutId: null,
    workoutName: "Free ride",
    summary: null,
  },
];

interface Backend {
  calls: { cmd: string; args: Record<string, unknown> }[];
  /** Per-command failures, thrown when the call arrives. */
  fail: Record<string, unknown>;
  /** Answers held until released, keyed by command or \`ride_summary:<id>\`. */
  hold: Record<string, Promise<void>>;
}
/**
 * `rides_list` and `ride_summary` behind `mockIPC`, with every refusal the core can give
 * (crates/crank-core/src/core.rs): `storage` when the store fails (`rides_list`: `store.rides()?`, core.rs `rides_list` (~2128));
 * core.rs `Core::ride_summary` (~2141) propagates it) and `not_found` "no ride \"<id>\"" for an id that is not stored
 * (`ride_summary`). `rides_list` has no other refusal. A stored summary the core cannot parse comes back
 * as `summary: null` (core.rs `record` (~2102)), like an open or aborted ride; both are in `RIDES`.
 */
function install(over: Partial<Backend> = {}): Backend {
  const b: Backend = { calls: [], fail: {}, hold: {}, ...over };
  mockIPC((cmd, payload) => {
    const args = (payload ?? {}) as Record<string, unknown>;
    b.calls.push({ cmd, args });
    if (cmd in b.fail) throw b.fail[cmd];
    const held = b.hold[cmd] ?? b.hold[`${cmd}:${String(args.id)}`];
    if (held) return held.then(() => answer(cmd, args));
    return answer(cmd, args);
  });
  const answer = (cmd: string, args: Record<string, unknown>) => {
    switch (cmd) {
      case "rides_list":
        return RIDES;
      case "ride_summary": {
        if (!("id" in args))
          throw "missing required key id for command `ride_summary`";
        const ride = RIDES.find((r) => r.id === args.id);
        if (!ride) throw apiError("not_found", `no ride "${args.id}"`);
        return ride;
      }
      default:
        throw `command ${cmd} not found`;
    }
  };
  return b;
}

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
    "This ride isn’t in your history.",
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
  expect(await screen.findByRole("list", { name: "Rides" })).toBeTruthy();
});

test("the list links each ride to its summary", async () => {
  install();
  navigate("history");
  open();
  const list = within(await screen.findByRole("list", { name: "Rides" }));
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
  expect(await screen.findByRole("list", { name: "Rides" })).toBeTruthy();
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
  expect(await screen.findByRole("list", { name: "Rides" })).toBeTruthy();
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
