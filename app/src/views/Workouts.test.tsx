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
  timeInZoneS: [300, 1200, 900, 600, 0, 0, 0],
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
}
/**
 * The core's workout commands behind `mockIPC`, as strict as the core: `workout_get` and `ride_load` answer
 * `not_found` for an id that is not in the library, `ride_load` is `conflict` while a ride is in progress, and
 * `ride_load` needs its `workoutId` argument (string or null) or Tauri rejects the call with a plain string.
 */
function install(over: Partial<Backend> = {}): Backend {
  const b: Backend = {
    calls: [],
    fail: {},
    compiled: { "w-ss": compiled(), "w-easy": compiled({ tss: 20 }) },
    rideActive: false,
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
        if (b.rideActive)
          throw apiError("conflict", "a ride is already in progress");
        if (id !== null && !(id in b.compiled))
          throw apiError("not_found", `no workout ${id}`);
        return {
          rideId: "ride-1",
          workoutId: id,
          hrLinked: false,
        } satisfies RideLoaded;
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
    ["Z5", "0:00"],
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
    /already in progress/,
  );
  expect(screen.getByTestId("route").textContent).toBe("workouts");
});
