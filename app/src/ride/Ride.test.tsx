// @vitest-environment jsdom
import {
  act,
  cleanup,
  render,
  renderHook,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { clearMocks } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import type { MetricId, StreamStatus, SubSample } from "../bindings";
import { navigate, useRoute } from "../router";
import { status as makeStatus } from "../stream/testBackend";
import { Ride, clock, targetNow } from "./Ride";
import { BLOCKS, installRide, type RideWorld } from "./rideBackend";
import { beginRide, useSession } from "./session";

const loaded = { rideId: "ride-1", workoutId: "w1", hrLinked: true };
const routes: string[] = [];
const Probe = () => {
  const r = useRoute();
  routes.push(`${r.view}:${r.sub ?? ""}`);
  return <output data-testid="route">{`${r.view}:${r.sub ?? ""}`}</output>;
};
const routeText = () => screen.getByTestId("route").textContent;

beforeEach(() => {
  routes.length = 0;
  navigate("ride");
});
afterEach(() => {
  cleanup();
  clearMocks();
});

type Rig = ReturnType<typeof installRide>;
async function open(over: Partial<RideWorld> = {}) {
  const rig = installRide(over);
  beginRide(loaded);
  render(
    <>
      <Probe />
      <Ride />
    </>,
  );
  await waitFor(() => expect(rig.base.liveSubs()).toBe(3));
  await waitFor(() => expect(rig.base.count("stream_snapshot")).toBe(1));
  return rig;
}
const subOf = (rig: Rig, metric: MetricId) => {
  const i = rig.base.calls
    .filter((c) => c.cmd === "stream_subscribe")
    .findIndex((c) => c.args.metric === metric);
  return rig.base.subOf(i);
};
const sample = (
  rig: Rig,
  metric: MetricId,
  v: number | null,
  t = 1,
): SubSample => ({
  sub: subOf(rig, metric),
  t,
  v,
});
const push = (
  rig: Rig,
  over: Partial<StreamStatus>,
  samples: SubSample[] = [],
) =>
  act(async () => {
    rig.base.truth.status = makeStatus(over);
    rig.base.push({ status: rig.base.truth.status, samples });
  });
const slot = (id: string) => document.querySelector(`[data-slot="${id}"]`);

describe("session", () => {
  test("one RideSession per load: useSession returns it, and the next load disposes it", () => {
    const first = beginRide(loaded);
    const a = renderHook(() => useSession());
    const b = renderHook(() => useSession());
    expect(a.result.current).toBe(first);
    expect(b.result.current).toBe(first);
    first.record(1, 10, 200);
    expect(first.power).toHaveLength(1);
    let second!: ReturnType<typeof beginRide>;
    act(() => {
      second = beginRide({ ...loaded, rideId: "ride-2" });
    });
    expect(second).not.toBe(first);
    expect(first.disposed).toBe(true);
    expect(first.power).toHaveLength(0);
    expect(second.disposed).toBe(false);
    expect(a.result.current).toBe(second);
  });
});

describe("Ride", () => {
  test("without a loaded ride it says so and offers Workouts", async () => {
    vi.resetModules();
    const fresh = await import("./Ride");
    render(<fresh.Ride />);
    expect(
      screen.getByRole("heading", { name: "No ride loaded" }),
    ).toBeTruthy();
    expect(screen.getByRole("button", { name: "Go to Workouts" })).toBeTruthy();
  });

  test("the tiles show the latest samples, the block countdown and the clocks", async () => {
    const rig = await open();
    await push(rig, { workoutS: 100, blockIndex: 1, blockElapsedS: 20 }, [
      sample(rig, "power", 242.4),
      sample(rig, "cadence", 91),
      sample(rig, "hr", 150),
    ]);
    expect(slot("power")?.textContent).toBe("242");
    expect(slot("target")?.textContent).toBe("250");
    expect(slot("cadence")?.textContent).toBe("91");
    expect(slot("hr")?.textContent).toBe("150");
    expect(slot("countdown")?.textContent).toBe("4:40");
    expect(slot("elapsed")?.textContent).toBe("1:40");
    expect(slot("left")?.textContent).toBe("8:20");
    expect(slot("block")?.textContent).toBe("WORK · block 2 of 3");
    await push(rig, { workoutS: 101, blockIndex: 1, blockElapsedS: 21 }, [
      sample(rig, "power", null, 2),
    ]);
    expect(slot("power")?.textContent).toBe("—");
  });

  test("the target is read off the compiled line at the block's elapsed time", () => {
    expect(targetNow(BLOCKS[0], 90)).toBe(140);
    expect(targetNow(BLOCKS[1], 10)).toBe(250);
    expect(targetNow(undefined, 0)).toBeNull();
  });

  test("Start calls ride_start and the core's status is shown", async () => {
    const rig = await open();
    expect(screen.getByTestId("state").textContent).toMatch(/Press Start/);
    await userEvent.click(screen.getByRole("button", { name: "Start" }));
    await waitFor(() => expect(rig.count("ride_start")).toBe(1));
    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toBe("Riding"),
    );
    await push(rig, { state: { kind: "paused", reason: "stopped_pedalling" } });
    expect(screen.getByTestId("state").textContent).toMatch(
      /waiting for you to pedal/,
    );
  });

  test.each([
    [{ trainer: "none" }, "device_unavailable", "no trainer"],
    [{ trainer: "connecting" }, "device_unavailable", "trainer not ready"],
    [{ cap: "requires_erg" }, "conflict", "cap_requires_erg"],
    [{ cap: "exceeds" }, "conflict", "cap_exceeded_target"],
    [{ recordFails: true }, "storage", "cannot open the record"],
    [{ phase: "none" }, "conflict", "no ride loaded"],
  ] as [Partial<RideWorld>, string, string][])(
    "a refused start (%o) shows the core's reason",
    async (over, code, message) => {
      const rig = await open({ ...over, phase: over.phase ?? "loaded" });
      // The rig's world is what the core says; a world that is not loaded still shows Start (status says idle).
      await userEvent.click(screen.getByRole("button", { name: "Start" }));
      const alert = await screen.findByRole("alert");
      expect(alert.textContent).toContain(message);
      expect(alert.getAttribute("data-code")).toBe(code);
      expect(rig.world.phase).not.toBe("active");
    },
  );

  test("pause, resume and skip call their commands; a refusal is shown, and the next command clears it", async () => {
    const rig = await open({ phase: "active" });
    await push(rig, { state: { kind: "riding" } });
    await userEvent.click(screen.getByRole("button", { name: "Pause" }));
    await waitFor(() => expect(rig.count("ride_pause")).toBe(1));
    await userEvent.click(
      await screen.findByRole("button", { name: "Resume" }),
    );
    await waitFor(() => expect(rig.count("ride_resume")).toBe(1));
    await userEvent.click(
      await screen.findByRole("button", { name: "Skip block" }),
    );
    await waitFor(() => expect(rig.count("ride_skip")).toBe(1));
    expect(rig.world.blockIdx).toBe(1);

    // The core stopped the ride behind the window's back: its refusal is the answer.
    rig.world.phase = "loaded";
    await userEvent.click(screen.getByRole("button", { name: "Pause" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("not valid in this state");
    rig.world.phase = "active";
    await userEvent.click(screen.getByRole("button", { name: "Skip block" }));
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
  });

  test("skipping the last block finishes the ride and the window leaves once", async () => {
    const rig = await open({ phase: "active", blockIdx: 2 });
    await push(rig, { state: { kind: "riding" }, blockIndex: 2 });
    await userEvent.click(screen.getByRole("button", { name: "Skip block" }));
    await waitFor(() => expect(routeText()).toBe("history:ride-1"));
    expect(rig.count("ride_skip")).toBe(1);
  });

  test("End asks first; Keep riding sends nothing, End ride ends and shows the summary route with the ride id", async () => {
    const rig = await open({ phase: "active" });
    await push(rig, { state: { kind: "riding" } });
    await userEvent.click(screen.getByRole("button", { name: "End" }));
    const dialog = screen.getByRole("alertdialog");
    expect(within(dialog).getByText("End this ride?")).toBeTruthy();
    expect(rig.count("ride_end")).toBe(0);
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Keep riding" }),
    );
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(rig.count("ride_end")).toBe(0);
    await userEvent.click(screen.getByRole("button", { name: "End" }));
    await userEvent.click(screen.getByRole("button", { name: "End ride" }));
    await waitFor(() => expect(routeText()).toBe("history:ride-1"));
    expect(rig.count("ride_end")).toBe(1);
    // The stream's `finished` and the end's answer both say so: the window still leaves once.
    await act(() => new Promise((r) => setTimeout(r, 40)));
    expect(routes.filter((r) => r.startsWith("history"))).toHaveLength(1);
  });

  test("ending a ride that never started unloads it and returns to Workouts", async () => {
    const rig = await open();
    await userEvent.click(screen.getByRole("button", { name: "End" }));
    expect(screen.getByRole("alertdialog").textContent).toMatch(/not started/);
    await userEvent.click(screen.getByRole("button", { name: "End ride" }));
    await waitFor(() => expect(routeText()).toBe("workouts:"));
    expect(rig.count("ride_end")).toBe(1);
  });

  test("a refused end shows the core's reason and stays", async () => {
    const rig = await open({ phase: "active" });
    await push(rig, { state: { kind: "riding" } });
    rig.world.phase = "none";
    await userEvent.click(screen.getByRole("button", { name: "End" }));
    await userEvent.click(screen.getByRole("button", { name: "End ride" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("no ride loaded");
    expect(routeText()).toBe("ride:");
  });

  test("state: finished navigates to the summary route exactly once, with the ride id", async () => {
    const rig = await open({ phase: "active" });
    await push(rig, { state: { kind: "riding" } });
    await push(rig, { state: { kind: "finished" } });
    await waitFor(() => expect(routeText()).toBe("history:ride-1"));
    await push(rig, { state: { kind: "finished" }, workoutS: 600 });
    await push(rig, { state: { kind: "finished" }, workoutS: 601 });
    expect(routes.filter((r) => r.startsWith("history"))).toHaveLength(1);
    expect(
      routes.filter((r) => r !== "ride:" && r !== "history:ride-1"),
    ).toEqual([]);
  });

  test("the device pill follows the links, with a word besides the colour", async () => {
    const rig = await open();
    await act(async () => {
      rig.base.push({
        links: [
          { device: "trainer", state: "ready" },
          { device: "hr", state: "lost" },
        ],
      });
    });
    const pill = screen.getByLabelText("Devices");
    expect(pill.textContent).toContain("NEO");
    expect(pill.textContent).toContain("ready");
    expect(pill.textContent).toContain("⚠ HR");
    expect(pill.textContent).toContain("lost");
  });

  test("the chart draws the target profile and the cursor advances with the workout clock", async () => {
    const rig = await open({ phase: "active" });
    await waitFor(() =>
      expect(screen.getAllByTestId("chart-block")).toHaveLength(3),
    );
    const cursor = () =>
      Number(screen.getByTestId("chart-cursor").getAttribute("x1"));
    await push(rig, { workoutS: 60 });
    expect(cursor()).toBeCloseTo(100);
    await push(rig, { workoutS: 300 });
    expect(cursor()).toBeCloseTo(500);
    expect(screen.queryByTestId("chart-power")).toBeNull();
    await push(rig, { workoutS: 301 }, [sample(rig, "power", 240, 5)]);
    await push(rig, { workoutS: 302 }, [sample(rig, "power", 250, 6)]);
    expect(
      screen.getByTestId("chart-power").getAttribute("points")?.split(" "),
    ).toHaveLength(2);
  });

  test("a sample delivered twice is one point of the history", async () => {
    const rig = await open({ phase: "active" });
    await push(rig, { workoutS: 10 }, [sample(rig, "power", 240, 5)]);
    await push(rig, { workoutS: 11 });
    await push(rig, { workoutS: 12 }, [sample(rig, "power", 240, 5)]);
    const session = renderHook(() => useSession()).result.current;
    expect(session?.power).toHaveLength(1);
  });

  test("no layout shift: a slot's size is set by its widest form, not by the value", async () => {
    const rig = await open();
    const sizes = () =>
      ["power", "target", "hr", "cadence", "countdown", "elapsed", "left"].map(
        (id) => (slot(id) as HTMLElement | null)?.style.width ?? "",
      );
    await push(rig, { workoutS: 5, blockElapsedS: 1 }, [
      sample(rig, "power", 5),
      sample(rig, "hr", 61),
    ]);
    const short = sizes();
    await push(rig, { workoutS: 3599, blockElapsedS: 99 }, [
      sample(rig, "power", 1234, 2),
      sample(rig, "hr", 188, 2),
    ]);
    expect(slot("power")?.textContent).toBe("1234");
    expect(sizes()).toEqual(short);
    expect(short.every((w) => /^\d+ch$/.test(w) || w === "")).toBe(true);
    // The tiles' own slots and the clocks reserve a width; the countdown is sized by the tier token.
    expect(short.slice(0, 4).every((w) => /^\d+ch$/.test(w))).toBe(true);
    expect(short[5]).toBe("7ch");
  });
});

test("clock formats m:ss and h:mm:ss", () => {
  expect(clock(5)).toBe("0:05");
  expect(clock(3599)).toBe("59:59");
  expect(clock(3600)).toBe("1:00:00");
  expect(clock(-3)).toBe("0:00");
});
