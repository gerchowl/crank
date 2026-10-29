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
const enabled = (name: string) =>
  !(screen.getByRole("button", { name }) as HTMLButtonElement).disabled;
async function open(
  over: Partial<RideWorld> = {},
  opts: { workoutId?: string | null; before?: (rig: Rig) => void } = {},
) {
  const rig = installRide(over);
  opts.before?.(rig);
  beginRide({
    ...loaded,
    workoutId: opts.workoutId === undefined ? "w1" : opts.workoutId,
  });
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
    first.record(2, 11, 210);
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
    expect(slot("plan")?.textContent).toBe("250");
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

  test("a Skip that would end the ride asks first; Keep riding sends nothing", async () => {
    const rig = await open({ phase: "active", blockIdx: 2 });
    await push(rig, { state: { kind: "riding" }, blockIndex: 2 });
    await userEvent.click(screen.getByRole("button", { name: "Skip block" }));
    const dialog = screen.getByRole("alertdialog");
    expect(dialog.textContent).toMatch(/skipping it ends the ride/);
    expect(rig.count("ride_skip")).toBe(0);
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Keep riding" }),
    );
    expect(rig.count("ride_skip")).toBe(0);
    expect(routeText()).toBe("ride:");
    await userEvent.click(screen.getByRole("button", { name: "Skip block" }));
    await userEvent.click(screen.getByRole("button", { name: "End ride" }));
    await waitFor(() => expect(routeText()).toBe("history:ride-1"));
    expect(rig.count("ride_skip")).toBe(1);
  });

  test("a second tap on Skip waits for the core's status to move past the block", async () => {
    const rig = await open({ phase: "active", publishes: false });
    await push(rig, { state: { kind: "riding" }, blockIndex: 0 });
    const skipBtn = () =>
      screen.getByRole("button", { name: "Skip block" }) as HTMLButtonElement;
    await userEvent.click(skipBtn());
    await waitFor(() => expect(rig.count("ride_skip")).toBe(1));
    await waitFor(() => expect(skipBtn().disabled).toBe(true));
    await userEvent.click(skipBtn());
    expect(rig.count("ride_skip")).toBe(1);
    await push(rig, { state: { kind: "riding" }, blockIndex: 1 });
    expect(skipBtn().disabled).toBe(false);
  });

  test("a refused Skip can be tried again", async () => {
    const rig = await open({ phase: "active" });
    await push(rig, { state: { kind: "riding" }, blockIndex: 0 });
    rig.world.phase = "loaded";
    await userEvent.click(screen.getByRole("button", { name: "Skip block" }));
    await screen.findByRole("alert");
    expect(
      (screen.getByRole("button", { name: "Skip block" }) as HTMLButtonElement)
        .disabled,
    ).toBe(false);
  });

  test("a double click on Start sends start and nothing else", async () => {
    const rig = await open();
    await userEvent.dblClick(screen.getByRole("button", { name: "Start" }));
    await waitFor(() => expect(rig.count("ride_start")).toBe(1));
    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toBe("Riding"),
    );
    expect(rig.count("ride_pause")).toBe(0);
    expect(
      (screen.getByRole("button", { name: "Start" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
  });

  test("a refused Start can be tried again", async () => {
    const rig = await open({ trainer: "connecting" });
    await userEvent.click(screen.getByRole("button", { name: "Start" }));
    await screen.findByRole("alert");
    rig.world.trainer = "ready";
    await userEvent.click(screen.getByRole("button", { name: "Start" }));
    await waitFor(() => expect(rig.world.phase).toBe("active"));
  });

  test("while a command is in flight nothing else can be sent", async () => {
    const rig = await open({ phase: "active" });
    await push(rig, { state: { kind: "riding" } });
    const release = rig.hold("ride_skip");
    await userEvent.click(screen.getByRole("button", { name: "Skip block" }));
    await waitFor(() => expect(rig.count("ride_skip")).toBe(1));
    for (const name of ["Pause", "Skip block", "End"])
      expect(
        (screen.getByRole("button", { name }) as HTMLButtonElement).disabled,
      ).toBe(true);
    await act(async () => release());
    await waitFor(() =>
      expect(
        (
          screen.getByRole("button", {
            name: "Skip block",
          }) as HTMLButtonElement
        ).disabled,
      ).toBe(false),
    );
  });

  test("the Pause button follows the core: a refused pause leaves Pause enabled and Resume off", async () => {
    const rig = await open({ phase: "active" });
    await push(rig, { state: { kind: "riding" } });
    rig.world.phase = "loaded";
    await userEvent.click(screen.getByRole("button", { name: "Pause" }));
    await screen.findByRole("alert");
    expect(enabled("Pause")).toBe(true);
    expect(enabled("Resume")).toBe(false);
  });

  test("a double tap on Pause sends one pause, even when the status arrives between the taps", async () => {
    const rig = await open({ phase: "active" });
    await push(rig, { state: { kind: "riding" } });
    const pauseBtn = screen.getByRole("button", { name: "Pause" });
    await userEvent.click(pauseBtn);
    await waitFor(() => expect(enabled("Resume")).toBe(true));
    await userEvent.click(pauseBtn);
    expect(rig.count("ride_pause")).toBe(1);
    expect(rig.count("ride_resume")).toBe(0);
  });

  test("a double tap on Resume sends one resume, even when the status arrives between the taps", async () => {
    const rig = await open({ phase: "active", userPause: true });
    await push(rig, { state: { kind: "paused", reason: "user" } });
    const resumeBtn = screen.getByRole("button", { name: "Resume" });
    await userEvent.click(resumeBtn);
    await waitFor(() => expect(enabled("Pause")).toBe(true));
    await userEvent.click(resumeBtn);
    expect(rig.count("ride_resume")).toBe(1);
    expect(rig.count("ride_pause")).toBe(0);
  });

  test("with the status frame still on its way, a second tap on Pause waits; a refusal re-enables it", async () => {
    const rig = await open({ phase: "active", publishes: false });
    await push(rig, { state: { kind: "riding" } });
    await userEvent.click(screen.getByRole("button", { name: "Pause" }));
    await waitFor(() => expect(rig.count("ride_pause")).toBe(1));
    await waitFor(() => expect(enabled("Pause")).toBe(false));
    await userEvent.click(screen.getByRole("button", { name: "Pause" }));
    expect(rig.count("ride_pause")).toBe(1);
    await push(rig, { state: { kind: "paused", reason: "user" } });
    expect(enabled("Resume")).toBe(true);
    // Resume refused: the control is free again.
    rig.world.phase = "loaded";
    await userEvent.click(screen.getByRole("button", { name: "Resume" }));
    await screen.findByRole("alert");
    expect(enabled("Resume")).toBe(true);
  });

  test("with the status frame still on its way, a second tap on Resume waits", async () => {
    const rig = await open({
      phase: "active",
      userPause: true,
      publishes: false,
    });
    await push(rig, { state: { kind: "paused", reason: "user" } });
    await userEvent.click(screen.getByRole("button", { name: "Resume" }));
    await waitFor(() => expect(rig.count("ride_resume")).toBe(1));
    await waitFor(() => expect(enabled("Resume")).toBe(false));
    await userEvent.click(screen.getByRole("button", { name: "Resume" }));
    expect(rig.count("ride_resume")).toBe(1);
  });

  test("with the status frame still on its way, a second tap on Start waits", async () => {
    const rig = await open({ publishes: false });
    await userEvent.click(screen.getByRole("button", { name: "Start" }));
    await waitFor(() => expect(rig.count("ride_start")).toBe(1));
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    expect(enabled("Start")).toBe(false);
    await userEvent.click(screen.getByRole("button", { name: "Start" }));
    expect(rig.count("ride_start")).toBe(1);
  });

  test("Skip waits for the plan: with it unanswered the ride cannot skip", async () => {
    let release = () => {};
    const rig = await open(
      { phase: "active" },
      { before: (r) => (release = r.hold("workout_get")) },
    );
    await push(rig, { state: { kind: "riding" }, blockIndex: 0 });
    expect(enabled("Skip block")).toBe(false);
    await act(async () => release());
    await waitFor(() => expect(enabled("Skip block")).toBe(true));
  });

  test("a free ride has no block to skip", async () => {
    const rig = await open(
      { phase: "active", blocks: [] },
      { workoutId: null },
    );
    await push(rig, { state: { kind: "riding" }, blockIndex: null });
    expect(enabled("Skip block")).toBe(false);
    await userEvent.dblClick(
      screen.getByRole("button", { name: "Skip block" }),
    );
    expect(rig.count("ride_skip")).toBe(0);
  });

  test.each([
    ["trainer_lost", "Pause"],
    ["stopped_pedalling", "Pause"],
    ["user", "Resume"],
  ] as const)(
    "a %s pause enables %s only: Resume is for the rider's own pause",
    async (reason, label) => {
      const rig = await open({ phase: "active" });
      await push(rig, { state: { kind: "paused", reason } });
      expect(enabled(label)).toBe(true);
      expect(enabled("Resume")).toBe(label === "Resume");
      expect(enabled("Pause")).toBe(label === "Pause");
    },
  );

  test.each([
    ["Start", {}, "idle"],
    ["Pause", { phase: "active" }, "riding"],
    ["Skip block", { phase: "active" }, "riding"],
    ["End", { phase: "active" }, "riding"],
  ] as [string, Partial<RideWorld>, string][])(
    "%s with the engine stopped shows the core's internal error",
    async (name, over, kind) => {
      const rig = await open({ ...over, engineStopped: true });
      await push(rig, { state: { kind } as StreamStatus["state"] });
      await userEvent.click(screen.getByRole("button", { name }));
      if (name === "End")
        await userEvent.click(screen.getByRole("button", { name: "End ride" }));
      const alert = await screen.findByRole("alert");
      expect(alert.getAttribute("data-code")).toBe("internal");
      expect(alert.textContent).toContain("engine stopped");
      expect(alert.textContent).toContain("The ride engine stopped:");
      expect(routeText()).toBe("ride:");
    },
  );

  test("Resume with the engine stopped shows the core's internal error", async () => {
    const rig = await open({
      phase: "active",
      userPause: true,
      engineStopped: true,
    });
    await push(rig, { state: { kind: "paused", reason: "user" } });
    await userEvent.click(screen.getByRole("button", { name: "Resume" }));
    expect((await screen.findByRole("alert")).textContent).toContain(
      "engine stopped",
    );
  });

  test("the plan tile is labelled Plan, not Target", async () => {
    await open();
    expect(slot("plan")?.closest("[data-tile]")?.textContent).toContain("PLAN");
    expect(document.body.textContent).not.toMatch(/TARGET/);
  });

  test("Escape closes the confirm and does not end the ride", async () => {
    const rig = await open({ phase: "active" });
    await push(rig, { state: { kind: "riding" } });
    await userEvent.click(screen.getByRole("button", { name: "End" }));
    expect(screen.getByRole("alertdialog")).toBeTruthy();
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(rig.count("ride_end")).toBe(0);
  });

  test("ending an unstarted ride disposes its session: a late Start finds no ride", async () => {
    const rig = await open();
    const session = renderHook(() => useSession()).result.current;
    await userEvent.click(screen.getByRole("button", { name: "End" }));
    await userEvent.click(screen.getByRole("button", { name: "End ride" }));
    await waitFor(() => expect(routeText()).toBe("workouts:"));
    expect(session?.disposed).toBe(true);
    expect(renderHook(() => useSession()).result.current).toBeNull();
    expect(rig.world.phase).toBe("none");
  });

  test("a new load re-opens the stream for the new ride", async () => {
    const rig = await open();
    expect(rig.base.count("open_stream")).toBe(1);
    act(() => {
      beginRide({ ...loaded, rideId: "ride-2" });
    });
    await waitFor(() => expect(rig.base.count("open_stream")).toBe(2));
    await waitFor(() => expect(rig.base.liveSubs()).toBe(3));
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
    // The effect fires again each time the state flips back to finished: still one navigation.
    for (const workoutS of [600, 601]) {
      await push(rig, { state: { kind: "riding" }, workoutS });
      await push(rig, { state: { kind: "finished" }, workoutS });
    }
    expect(routes.filter((r) => r.startsWith("history"))).toHaveLength(1);
    expect(
      routes.filter((r) => r !== "ride:" && r !== "history:ride-1"),
    ).toEqual([]);
  });

  test("a finished ride cannot be ended again", async () => {
    const rig = await open({ phase: "active" });
    await push(rig, { state: { kind: "finished" } });
    expect(
      (screen.getByRole("button", { name: "End" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
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
      ["power", "plan", "hr", "cadence", "countdown", "elapsed", "left"].map(
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
