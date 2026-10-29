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
    for (const name of ["Pause", "Skip block"])
      expect(
        (screen.getByRole("button", { name }) as HTMLButtonElement).disabled,
      ).toBe(true);
    // End is the way out and is not held by it (#327).
    expect(enabled("End")).toBe(true);
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
    await push(rig, {
      state: { kind: "paused", reason: "user" },
      userPaused: true,
    });
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
    await push(rig, {
      state: { kind: "paused", reason: "user" },
      userPaused: true,
    });
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
    await push(rig, {
      state: { kind: "paused", reason: "user" },
      userPaused: true,
    });
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
      await push(rig, {
        state: { kind: "paused", reason },
        userPaused: reason === "user",
        trainerLostSinceS: reason === "trainer_lost" ? 5 : null,
      });
      expect(enabled(label)).toBe(true);
      expect(enabled("Resume")).toBe(label === "Resume");
      expect(enabled("Pause")).toBe(label === "Pause");
    },
  );

  test.each([
    ["Start", {}, "idle"],
    ["Pause", { phase: "active" }, "riding"],
    ["Skip block", { phase: "active" }, "riding"],
  ] as [string, Partial<RideWorld>, string][])(
    "%s with the engine stopped shows the core's internal error",
    async (name, over, kind) => {
      const rig = await open({ ...over, engineStopped: true });
      await push(rig, { state: { kind } as StreamStatus["state"] });
      await userEvent.click(screen.getByRole("button", { name }));
      const alert = await screen.findByRole("alert");
      expect(alert.getAttribute("data-code")).toBe("internal");
      expect(alert.textContent).toContain("engine stopped");
      expect(alert.textContent).toContain("The ride engine stopped:");
      expect(routeText()).toBe("ride:");
    },
  );

  test("a wedged engine: a command shows the core's refusal, the view says the engine is not answering, and End still leaves the ride (#351)", async () => {
    const rig = await open({ phase: "active" });
    await push(rig, { state: { kind: "riding" } });
    rig.wedge();
    await userEvent.click(screen.getByRole("button", { name: "Pause" }));
    const refusal = await waitFor(() => {
      const el = document.querySelector('[data-code="engine_unresponsive"]');
      expect(el).not.toBeNull();
      return el as HTMLElement;
    });
    expect(refusal.textContent).toContain("The ride engine is not answering:");
    expect(screen.queryByTestId("engine-unresponsive")).toBeNull();
    // the next frame's ack meets the wedged engine: the stream's error, shown once, not retried
    await push(rig, { state: { kind: "riding" } });
    const banner = await screen.findByTestId("engine-unresponsive");
    expect(banner.getAttribute("role")).toBe("alert");
    expect(banner.textContent).toContain("End still ends the ride");
    expect(rig.base.count("open_stream")).toBe(1);
    await userEvent.click(screen.getByRole("button", { name: "End" }));
    await userEvent.click(screen.getByRole("button", { name: "End ride" }));
    await waitFor(() => expect(routeText()).toBe("history:ride-1"));
  });

  test("End with the engine stopped still leaves the ride: the core ends it by force (#331)", async () => {
    const rig = await open({ phase: "active", engineStopped: true });
    await push(rig, { state: { kind: "riding" } });
    await userEvent.click(screen.getByRole("button", { name: "End" }));
    await userEvent.click(screen.getByRole("button", { name: "End ride" }));
    await waitFor(() => expect(routeText()).toBe("history:ride-1"));
    expect(screen.queryByRole("alert")).toBeNull();
  });

  test("Resume with the engine stopped shows the core's internal error", async () => {
    const rig = await open({
      phase: "active",
      userPause: true,
      engineStopped: true,
    });
    await push(rig, {
      state: { kind: "paused", reason: "user" },
      userPaused: true,
    });
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

describe("loss and reconnect states (#100 MVP, DESIGN §6.5)", () => {
  const publish = (rig: Rig, change: Partial<RideWorld>) =>
    act(async () => {
      Object.assign(rig.world, change);
      rig.publish();
    });
  const rideWith = async (samples = true) => {
    const rig = await open({ phase: "active" });
    await push(rig, { workoutS: 100, blockIndex: 1, blockElapsedS: 65 }, [
      ...(samples
        ? [
            sample(rig, "power", 240),
            sample(rig, "cadence", 90),
            sample(rig, "hr", 150),
          ]
        : []),
    ]);
    return rig;
  };

  test("trainer lost: the banner names the pause point, power and cadence say —, HR stays", async () => {
    const rig = await rideWith();
    await publish(rig, { trainerLost: true, blockIdx: 1 });
    const banner = screen.getByTestId("loss-trainer");
    expect(banner.textContent).toContain("NEO lost");
    expect(banner.textContent).toContain("paused at");
    expect(banner.textContent).toContain("reconnecting");
    expect(banner.textContent).not.toContain("attempt");
    expect(slot("power")?.textContent).toBe("—");
    expect(slot("cadence")?.textContent).toBe("—");
    expect(slot("hr")?.textContent).toBe("150");
    expect(screen.queryByTestId("loss-hr")).toBeNull();
    expect(
      screen.getByText(/NEO/, { selector: ".pill-half" }).textContent,
    ).toContain("⚠");
  });

  test("no data on a link that is still ready: the banner does not say reconnecting", async () => {
    const rig = await rideWith();
    await publish(rig, { trainerSilent: true, blockIdx: 1 });
    const banner = screen.getByTestId("loss-trainer").textContent ?? "";
    expect(banner).toContain("No data from the NEO");
    expect(banner).not.toContain("reconnecting");
    expect(banner).not.toContain("NEO lost");
    await publish(rig, { trainerSilent: false, trainerLost: true });
    expect(screen.getByTestId("loss-trainer").textContent).toContain(
      "NEO lost",
    );
    expect(screen.getByTestId("loss-trainer").textContent).toContain(
      "reconnecting",
    );
  });

  test("no links frame yet: the trainer-lost banner is neutral, not 'reconnecting'", async () => {
    const rig = await open({ phase: "active" });
    await push(rig, {
      state: { kind: "paused", reason: "trainer_lost" },
      trainerLostSinceS: 5,
    });
    const banner = screen.getByTestId("loss-trainer").textContent ?? "";
    expect(banner).toContain("No data from the NEO");
    expect(banner).not.toContain("reconnecting");
  });

  test("stopped pedalling is not a trainer loss", async () => {
    const rig = await rideWith();
    await publish(rig, { stoppedPedalling: true, blockIdx: 1 });
    expect(screen.queryByTestId("loss-trainer")).toBeNull();
    expect(slot("power")?.textContent).toBe("240");
  });

  test("ERG released is not shown while paused (a deliberate choice)", async () => {
    const rig = await rideWith();
    await publish(rig, { ergReleased: true, blockIdx: 1 });
    expect(screen.getByTestId("loss-erg")).toBeTruthy();
    // The core keeps the release set across a stop: paused with `ergReleased: true` is a real shape.
    await publish(rig, { stoppedPedalling: true });
    expect(rig.base.truth.status.ergReleased).toBe(true);
    expect(rig.base.truth.status.state.kind).toBe("paused");
    expect(screen.queryByTestId("loss-erg")).toBeNull();
  });

  test("HR lost: the ride goes on, the HR tile says — and the banner is the HR note", async () => {
    const rig = await rideWith();
    await publish(rig, { hrLost: true, blockIdx: 1 });
    expect(screen.getByTestId("loss-hr").textContent).toContain("HR lost");
    expect(screen.queryByTestId("loss-trainer")).toBeNull();
    expect(slot("hr")?.textContent).toBe("—");
    expect(slot("power")?.textContent).toBe("240");
    expect(screen.getByTestId("state").textContent).toBe("Riding");
  });

  test("both lost: one banner for both", async () => {
    const rig = await rideWith();
    await publish(rig, { trainerLost: true, hrLost: true, blockIdx: 1 });
    expect(screen.getByTestId("loss-trainer").textContent).toContain(
      "NEO and HR lost",
    );
    expect(screen.queryByTestId("loss-hr")).toBeNull();
    for (const id of ["power", "cadence", "hr"])
      expect(slot(id)?.textContent).toBe("—");
  });

  test("ERG released: the row says to spin up", async () => {
    const rig = await rideWith();
    await publish(rig, { ergReleased: true, blockIdx: 1 });
    expect(screen.getByTestId("loss-erg").textContent).toContain(
      "ERG released — spin up",
    );
    await publish(rig, { ergReleased: false });
    expect(screen.queryByTestId("loss-erg")).toBeNull();
  });

  test("the states end with the loss: reconnected shows no banner (the core reports no soft start, #329)", async () => {
    const rig = await rideWith();
    await publish(rig, { trainerLost: true, blockIdx: 1 });
    await publish(rig, { trainerLost: false });
    expect(screen.queryByTestId("loss-trainer")).toBeNull();
    expect(screen.getByTestId("state").textContent).toBe("Riding");
  });

  test("never inferred: no trainer-lost flag and an empty power tile show no lost banner", async () => {
    const rig = await rideWith(false);
    await push(rig, { state: { kind: "riding" } }, [
      sample(rig, "power", null),
      sample(rig, "cadence", null),
    ]);
    expect(slot("power")?.textContent).toBe("—");
    expect(screen.queryByTestId("loss-trainer")).toBeNull();
    expect(screen.queryByTestId("loss-hr")).toBeNull();
  });

  test("no loss rows without a live ride: a lost link before Start or after the end is not a loss", async () => {
    const rig = await open({ phase: "loaded" });
    await act(async () => {
      rig.base.push({ links: [{ device: "hr", state: "lost" }] });
    });
    expect(screen.queryByTestId("loss-hr")).toBeNull();
  });
});

describe("End stays usable (#327)", () => {
  test("a ride command that never answers does not hold End", async () => {
    const rig = await open({ phase: "active" });
    await push(rig, { state: { kind: "riding" } });
    rig.hold("ride_pause"); // never released
    await userEvent.click(screen.getByRole("button", { name: "Pause" }));
    await waitFor(() => expect(rig.count("ride_pause")).toBe(1));
    expect(enabled("Resume")).toBe(false);
    expect(enabled("End")).toBe(true);
    await userEvent.click(screen.getByRole("button", { name: "End" }));
    await userEvent.click(screen.getByRole("button", { name: "End ride" }));
    await waitFor(() => expect(routeText()).toBe("history:ride-1"));
    expect(rig.count("ride_end")).toBe(1);
  });

  test("an End that never answers can be asked again; the window still leaves once", async () => {
    // The engine has not processed the first End: no status frame follows it.
    const rig = await open({ phase: "active", publishes: false });
    await push(rig, { state: { kind: "riding" } });
    rig.hold("ride_end"); // the first never answers
    await userEvent.click(screen.getByRole("button", { name: "End" }));
    await userEvent.click(screen.getByRole("button", { name: "End ride" }));
    await waitFor(() => expect(rig.count("ride_end")).toBe(1));
    expect(enabled("End")).toBe(true);
    await userEvent.click(screen.getByRole("button", { name: "End" }));
    await userEvent.click(screen.getByRole("button", { name: "End ride" }));
    await waitFor(() => expect(routeText()).toBe("history:ride-1"));
    expect(routes.filter((r) => r.startsWith("history"))).toHaveLength(1);
  });

  test("Pause is free again once the status shows the pause was lifted by someone else", async () => {
    const rig = await open({ phase: "active" });
    await push(rig, { state: { kind: "riding" } });
    await userEvent.click(screen.getByRole("button", { name: "Pause" }));
    await waitFor(() => expect(enabled("Resume")).toBe(true));
    // Not this screen: the coach resumes the rider pause.
    await act(async () => {
      rig.world.userPause = false;
      rig.publish();
    });
    await waitFor(() => expect(enabled("Pause")).toBe(true));
  });

  test("an external pause after a screen Resume: Pause is enabled again", async () => {
    const rig = await open({ phase: "active", userPause: true });
    await push(rig, {
      state: { kind: "paused", reason: "user" },
      userPaused: true,
    });
    await userEvent.click(screen.getByRole("button", { name: "Resume" }));
    await waitFor(() => expect(enabled("Pause")).toBe(true));
    await act(async () => {
      rig.world.userPause = true;
      rig.publish();
    });
    await waitFor(() => expect(enabled("Resume")).toBe(true));
    await act(async () => {
      rig.world.userPause = false;
      rig.publish();
    });
    await waitFor(() => expect(enabled("Pause")).toBe(true));
  });
});

describe("reconnect rows and the rider pause under a loss (#100, #327 item 3; DESIGN §6.5, §6.6)", () => {
  const publish = (rig: Rig, change: Partial<RideWorld>) =>
    act(async () => {
      Object.assign(rig.world, change);
      rig.publish();
    });
  const text = (id: string) => screen.queryByTestId(id)?.textContent ?? null;
  const lostRide = async (over: Partial<RideWorld> = {}) => {
    const rig = await open({ phase: "active" });
    await publish(rig, {
      trainerLost: true,
      sessionS: 100,
      lostSinceS: 100,
      blockIdx: 1,
      ...over,
    });
    return rig;
  };

  test("trainer lost: the attempt is the driver's, and 'reconnecting' needs a link that is not ready", async () => {
    const rig = await lostRide();
    expect(text("loss-trainer")).toContain("reconnecting");
    expect(text("loss-trainer")).not.toContain("attempt");
    await publish(rig, { attempt: 3 });
    expect(text("loss-trainer")).toContain("reconnecting (attempt 3)");
    // 8 silent ticks on a link that is still ready: lost, but not reconnecting, so no attempt either.
    await publish(rig, { trainerLost: false, trainerSilent: true });
    expect(text("loss-trainer")).toContain("No data from the NEO");
    expect(text("loss-trainer")).not.toContain("reconnecting");
    expect(text("loss-trainer")).not.toContain("attempt");
  });

  test("lost for 29 s: no what-to-try row; at 30 s on the session clock it adds the text and End…", async () => {
    const rig = await lostRide();
    await publish(rig, { sessionS: 129 });
    expect(screen.queryByTestId("loss-help")).toBeNull();
    expect(screen.queryByRole("button", { name: "End…" })).toBeNull();
    await publish(rig, { sessionS: 130 });
    expect(text("loss-help")).toContain("power-cycle NEO · close Zwift");
    expect(screen.getByRole("button", { name: "End…" })).toBeTruthy();
  });

  test("lost for 29 s from a later stamp: the duration counts from the stamp, not from the ride", async () => {
    await lostRide({ sessionS: 130, lostSinceS: 101 });
    expect(screen.queryByTestId("loss-help")).toBeNull();
    expect(text("lost-for")).toBe("0:29");
  });

  test("the banner shows how long the trainer has been lost, on the session clock (DESIGN §5.2)", async () => {
    const rig = await lostRide({ sessionS: 112 });
    expect(text("lost-for")).toBe("0:12");
    expect(text("loss-trainer")).toContain("NEO lost 0:12 ·");
    await publish(rig, { sessionS: 175 });
    expect(text("lost-for")).toBe("1:15");
  });

  test("a mock that rewinds the session clock throws: the core cannot send it", async () => {
    const rig = await lostRide({ sessionS: 120 });
    expect(() => {
      Object.assign(rig.world, { sessionS: 110 });
      rig.publish();
    }).toThrow("session clock went backwards");
  });

  test("End… opens the End dialog with Keep riding focused; the ride ends only on End ride", async () => {
    const rig = await lostRide({ sessionS: 140 });
    await userEvent.click(screen.getByRole("button", { name: "End…" }));
    expect(screen.getByRole("alertdialog")).toBeTruthy();
    expect(document.activeElement?.textContent).toBe("Keep riding");
    expect(rig.count("ride_end")).toBe(0);
    await userEvent.click(screen.getByRole("button", { name: "End ride" }));
    await waitFor(() => expect(routeText()).toBe("history:ride-1"));
    expect(rig.count("ride_end")).toBe(1);
  });

  test("the loss rows are gone for a finished ride (fields are null outside Active)", async () => {
    const rig = await lostRide({ sessionS: 140 });
    await publish(rig, { phase: "finished" });
    expect(screen.queryByTestId("loss-help")).toBeNull();
    expect(screen.queryByTestId("loss-trainer")).toBeNull();
  });

  test.each([
    [119, false],
    [120, true],
  ])(
    "trainer back after a %i s loss: the Back-after row shown: %s",
    async (gapS, shown) => {
      const rig = await open({ phase: "active" });
      await publish(rig, {
        pausedAfterLoss: true,
        gapS,
        reconnectedAtS: 500,
        sessionS: 520,
      });
      expect(screen.queryByTestId("loss-trainer")).toBeNull();
      if (shown) {
        expect(text("re-entry")).toBe(`↻ Back after ${clock(gapS)}`);
      } else expect(screen.queryByTestId("re-entry")).toBeNull();
    },
  );

  test("the re-entry row states the fact only: no policy words until the engine reports a re-entry plan (#50)", async () => {
    const rig = await open({ phase: "active" });
    // Over 15 min DESIGN §6.6 says "no resume"; the engine does none of it, so the row says none of it either.
    await publish(rig, {
      pausedAfterLoss: true,
      gapS: 1000,
      reconnectedAtS: 5,
    });
    const row = text("re-entry") ?? "";
    expect(row).toBe("↻ Back after 16:40");
    expect(row).not.toMatch(/warm|restart|start|resume|interval|then/i);
  });

  test("soft start and re-entry both reported: both rows show, each is its own field", async () => {
    const rig = await open({ phase: "active" });
    await publish(rig, {
      pausedAfterLoss: true,
      gapS: 200,
      reconnectedAtS: 300,
      sessionS: 302,
      softStartW: 253,
    });
    expect(text("re-entry")).toBe("↻ Back after 3:20");
    expect(text("soft-start")).toBe("Reconnected · easing back in to 253 W");
    expect(text("neo-back")).toBe("NEO back · 3:20 gap");
  });

  test("the re-entry text ends when the ride is riding again", async () => {
    const rig = await open({ phase: "active" });
    await publish(rig, { pausedAfterLoss: true, gapS: 190, reconnectedAtS: 5 });
    expect(text("re-entry")).toContain("↻ Back after 3:10");
    await publish(rig, { pausedAfterLoss: false });
    expect(screen.queryByTestId("re-entry")).toBeNull();
  });

  test("reconnected soft start: 'easing back in to <W> W' while the core reports a target", async () => {
    const rig = await open({ phase: "active" });
    await publish(rig, { gapS: 14, reconnectedAtS: 200, sessionS: 202 });
    expect(screen.queryByTestId("soft-start")).toBeNull();
    await publish(rig, { softStartW: 253 });
    expect(text("soft-start")).toBe("Reconnected · easing back in to 253 W");
    await publish(rig, { softStartW: null, sessionS: 203 });
    expect(screen.queryByTestId("soft-start")).toBeNull();
  });

  test("the 'NEO back · m:ss gap' chip lives 10 s on the session clock", async () => {
    const rig = await open({ phase: "active" });
    await publish(rig, { gapS: 14, reconnectedAtS: 200, sessionS: 200 });
    expect(text("neo-back")).toBe("NEO back · 0:14 gap");
    await publish(rig, { sessionS: 209 });
    expect(text("neo-back")).toBe("NEO back · 0:14 gap");
    await publish(rig, { sessionS: 210 });
    expect(text("neo-back")).toBe("");
    // The next loss clears the chip (the core clears the gap with it).
    await publish(rig, { sessionS: 212, trainerLost: true, lostSinceS: 212 });
    expect(text("neo-back")).toBe("");
  });

  test("the chip's slot is always there, so it moves nothing", async () => {
    await open({ phase: "active" });
    expect(screen.getByTestId("neo-back")).toBeTruthy();
    expect(screen.getByTestId("neo-back").style.width).toBe("20ch");
  });

  test("#327.3: under a loss the core still holds the rider pause: Resume follows it and Pause is off with a reason", async () => {
    const rig = await lostRide({ userPause: true, sessionS: 105 });
    expect(rig.base.truth.status.state).toEqual({
      kind: "paused",
      reason: "trainer_lost",
    });
    expect(rig.base.truth.status.userPaused).toBe(true);
    expect(enabled("Resume")).toBe(true);
    expect(enabled("Pause")).toBe(false);
    expect(
      screen.getByRole("button", { name: "Pause" }).getAttribute("title"),
    ).toBe("Already paused");
    await userEvent.click(screen.getByRole("button", { name: "Resume" }));
    await waitFor(() => expect(rig.count("ride_resume")).toBe(1));
    // The rider pause is lifted; the loss goes on.
    await waitFor(() => expect(enabled("Pause")).toBe(true));
    expect(enabled("Resume")).toBe(false);
    expect(screen.getByTestId("loss-trainer")).toBeTruthy();
    expect(rig.count("ride_pause")).toBe(0);
  });

  test("#327.3: under a loss with no rider pause, Pause is the only one and the core takes it", async () => {
    const rig = await lostRide();
    expect(enabled("Pause")).toBe(true);
    expect(enabled("Resume")).toBe(false);
    await userEvent.click(screen.getByRole("button", { name: "Pause" }));
    await waitFor(() => expect(enabled("Resume")).toBe(true));
    expect(enabled("Pause")).toBe(false);
    expect(rig.count("ride_pause")).toBe(1);
  });

  test("#327.3: Pause under a held rider pause is never sent, so the core is never asked for it twice", async () => {
    const rig = await lostRide({ userPause: true });
    await userEvent.click(screen.getByRole("button", { name: "Pause" }));
    expect(rig.count("ride_pause")).toBe(0);
  });

  test("never inferred: the pause reason, the links and the clock alone show no loss rows", async () => {
    const rig = await open({ phase: "active" });
    // `trainer_lost` with every loss field null (a shape the core makes only when no loss is reported): nothing to show.
    await act(async () => {
      rig.base.truth.status = makeStatus({
        state: { kind: "paused", reason: "trainer_lost" },
        sessionS: 900,
      });
      rig.base.push({
        status: rig.base.truth.status,
        links: [{ device: "trainer", state: "lost" }],
      });
    });
    for (const id of ["loss-trainer", "loss-help", "re-entry", "soft-start"])
      expect(screen.queryByTestId(id)).toBeNull();
    expect(text("neo-back")).toBe("");
    expect(text("state")).toBe("Paused: waiting for you to pedal");
    // Riding, long after a reconnect that the core reports no gap for: no chip, no rows.
    await push(rig, { state: { kind: "riding" }, sessionS: 901 });
    expect(text("neo-back")).toBe("");
    expect(screen.queryByTestId("re-entry")).toBeNull();
  });

  test("never inferred: a lost stamp with no session clock gives no duration, so no ≥ 30 s row", async () => {
    const rig = await open({ phase: "active" });
    await push(rig, {
      state: { kind: "paused", reason: "trainer_lost" },
      trainerLostSinceS: 1,
      sessionS: null,
    });
    expect(screen.getByTestId("loss-trainer")).toBeTruthy();
    expect(screen.queryByTestId("loss-help")).toBeNull();
  });
});
