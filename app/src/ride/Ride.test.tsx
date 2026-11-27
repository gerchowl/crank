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
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { commands } from "../bindings";
import type { LiveValues, StreamStatus } from "../bindings";
import { navigate, useRoute } from "../router";
import { status as makeStatus } from "../stream/testBackend";
import { UiClient, WATCHDOG_MS, type Timers } from "../stream/ui";
import { UiProvider } from "../stream/uiHooks";
import { rideView } from "../stream/uiBackend";
import { Ride, clock, targetNow } from "./Ride";
import { forgetAthleteModel, refreshAthleteModel } from "../athleteHr";
import { estimate, install as installAthlete } from "../views/athleteBackend";
import {
  BLOCKS,
  PRIOR_SNAPSHOT,
  block,
  installRide,
  type RideWorld,
} from "./rideBackend";
import { resetLeft } from "./session";

/** A clock the watchdog cannot fire on: tests that are not about it. */
const neverClock: Timers = {
  setTimeout: () => 0,
  clearTimeout: () => {},
};

/** A fake clock the engine-unresponsive test moves by hand (same shape as stream/ui.test.tsx's). */
class FakeClock implements Timers {
  now = 0;
  private nextId = 1;
  private readonly due = new Map<number, { at: number; fn: () => void }>();
  setTimeout(fn: () => void, ms: number): unknown {
    const id = this.nextId++;
    this.due.set(id, { at: this.now + ms, fn });
    return id;
  }
  clearTimeout(handle: unknown): void {
    this.due.delete(handle as number);
  }
  async advance(ms: number): Promise<void> {
    const end = this.now + ms;
    for (;;) {
      const first = [...this.due.entries()]
        .filter(([, d]) => d.at <= end)
        .sort((a, b) => a[1].at - b[1].at)[0];
      if (!first) break;
      this.due.delete(first[0]);
      this.now = first[1].at;
      first[1].fn();
      await act(async () => {
        for (let i = 0; i < 5; i++) await Promise.resolve();
      });
    }
    this.now = end;
    await act(async () => {
      for (let i = 0; i < 5; i++) await Promise.resolve();
    });
  }
}

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
  resetLeft();
  cleanup();
  clearMocks();
});

type Rig = ReturnType<typeof installRide>;
const enabled = (name: string) =>
  !(screen.getByRole("button", { name }) as HTMLButtonElement).disabled;

async function open(
  over: Partial<RideWorld> = {},
  opts: {
    workoutId?: string | null;
    before?: (rig: Rig) => void;
    clock?: Timers;
  } = {},
) {
  const world: Partial<RideWorld> = { ...over };
  if (opts.workoutId !== undefined) world.workoutId = opts.workoutId;
  const rig = installRide(world);
  opts.before?.(rig);
  const client = new UiClient(opts.clock ?? neverClock);
  render(
    <UiProvider client={client}>
      <Probe />
      <Ride />
    </UiProvider>,
  );
  await waitFor(() => expect(rig.ui.count("ui_open")).toBe(1));
  await waitFor(() => expect(client.snapshot.received).toBe(true));
  return { rig, client };
}

/** A status (and optionally the big-number values) the next frame will carry. The setLive runs first because its
 * `show()` rebuilds the frame's status from the last publish-ed one and so would clobber a preceding setStatus. */
const push = (
  rig: Rig,
  over: Partial<StreamStatus> = {},
  live?: Partial<LiveValues>,
) =>
  act(async () => {
    if (live !== undefined) rig.setLive(live);
    if (Object.keys(over).length > 0) rig.ui.setStatus(over);
  });

const slot = (id: string) => document.querySelector(`[data-slot="${id}"]`);

describe("Ride", () => {
  test("without a loaded ride it says so and offers Workouts", async () => {
    const rig = installRide({ phase: "none" });
    const client = new UiClient(neverClock);
    render(
      <UiProvider client={client}>
        <Ride />
      </UiProvider>,
    );
    await waitFor(() => expect(rig.ui.count("ui_open")).toBe(1));
    await waitFor(() => expect(client.snapshot.received).toBe(true));
    expect(
      screen.getByRole("heading", { name: "No ride loaded" }),
    ).toBeTruthy();
    expect(screen.getByRole("button", { name: "Go to Workouts" })).toBeTruthy();
  });

  test("a refused ui_open is shown on the ride route, not an endless Loading; Retry opens the channel again", async () => {
    // `storage`: the keeper cannot read the pairings (core.rs `Core::ui_open`). No frame ever arrives.
    const rig = installRide({}, { keeperStorage: true });
    const client = new UiClient(neverClock);
    render(
      <UiProvider client={client}>
        <Ride />
      </UiProvider>,
    );
    expect(
      await screen.findByRole("heading", {
        name: "Live data is not available",
      }),
    ).toBeTruthy();
    expect(screen.getByRole("alert").textContent).toBe(
      "database is locked. Trying again every 2 s.",
    );
    expect(
      screen.queryByRole("heading", { name: "Loading the ride…" }),
    ).toBeNull();
    rig.ui.truth.keeperStorage = false;
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(await screen.findByTestId("ride")).toBeTruthy();
    expect(rig.ui.count("ui_open")).toBe(2);
  });

  test("the page without a ride shows a stalled channel too", async () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    const rig = installRide({ phase: "none" });
    let due: (() => void) | null = null;
    const clock: Timers = {
      setTimeout: (fn) => {
        due = fn;
        return 1;
      },
      clearTimeout: () => {
        due = null;
      },
    };
    const client = new UiClient(clock);
    render(
      <UiProvider client={client}>
        <Ride />
      </UiProvider>,
    );
    expect(
      await screen.findByRole("heading", { name: "No ride loaded" }),
    ).toBeTruthy();
    rig.ui.stall();
    act(() => due?.());
    expect(screen.getByTestId("channel-stale").textContent).toBe(
      "⚠ Live data stopped: what is shown may be out of date. Reconnecting…",
    );
    rig.ui.resume();
    await waitFor(() =>
      expect(screen.queryByTestId("channel-stale")).toBeNull(),
    );
  });

  test("the tiles show the latest samples, the block countdown and the clocks", async () => {
    const { rig } = await open();
    await push(
      rig,
      { workoutS: 100, blockIndex: 1, blockElapsedS: 20 },
      { powerW: 242.4, cadenceRpm: 91, hrBpm: 150 },
    );
    expect(slot("power")?.textContent).toBe("242");
    expect(slot("plan")?.textContent).toBe("Plan 250 W");
    expect(slot("cadence")?.textContent).toBe("91");
    expect(slot("hr")?.textContent).toBe("150");
    expect(slot("countdown")?.textContent).toBe("4:40");
    expect(slot("elapsed")?.textContent).toBe("1:40");
    expect(slot("left")?.textContent).toBe("8:20");
    expect(slot("block")?.textContent).toBe("WORK · block 2 of 3");
    await push(rig, {}, { powerW: null });
    expect(slot("power")?.textContent).toBe("—");
  });

  test("the target is read off the compiled line at the block's elapsed time", () => {
    expect(targetNow(BLOCKS[0], 90)).toBe(140);
    expect(targetNow(BLOCKS[1], 10)).toBe(250);
    expect(targetNow(undefined, 0)).toBeNull();
  });

  test("Start calls ride_start and the core's status is shown", async () => {
    const { rig } = await open();
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

  // The trainer-is-not-ready and "no ride loaded" rows the old matrix carried now live in the Ready sheet (its Start
  // is disabled, ReadySheet.test.tsx) and in the "no ride loaded" Screen; they cannot be reached from a tap on Start.
  test.each([
    [{ cap: "requires_erg" }, "conflict", "cap_requires_erg"],
    [{ cap: "exceeds" }, "conflict", "cap_exceeded_target"],
    [{ recordFails: true }, "storage", "cannot open the record"],
  ] as [Partial<RideWorld>, string, string][])(
    "a refused start (%o) shows the core's reason",
    async (over, code, message) => {
      const { rig } = await open({ ...over, phase: over.phase ?? "loaded" });
      await userEvent.click(screen.getByRole("button", { name: "Start" }));
      const alert = await screen.findByRole("alert");
      expect(alert.textContent).toContain(message);
      expect(alert.getAttribute("data-code")).toBe(code);
      expect(rig.world.phase).not.toBe("active");
    },
  );

  test("pause, resume and skip call their commands; a refusal is shown, and the next command clears it", async () => {
    const { rig } = await open({ phase: "active" });
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
    const { rig } = await open({ phase: "active", blockIdx: 2 });
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
    const { rig } = await open({ phase: "active", publishes: false });
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
    const { rig } = await open({ phase: "active" });
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
    const { rig } = await open();
    await userEvent.dblClick(screen.getByRole("button", { name: "Start" }));
    await waitFor(() => expect(rig.count("ride_start")).toBe(1));
    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toBe("Riding"),
    );
    expect(rig.count("ride_pause")).toBe(0);
    expect(screen.queryByRole("button", { name: "Start" })).toBeNull();
  });

  test("a refused Start can be tried again", async () => {
    const { rig } = await open({ recordFails: true });
    await userEvent.click(screen.getByRole("button", { name: "Start" }));
    await screen.findByRole("alert");
    rig.world.recordFails = false;
    await userEvent.click(screen.getByRole("button", { name: "Start" }));
    await waitFor(() => expect(rig.world.phase).toBe("active"));
  });

  test("while a command is in flight nothing else can be sent", async () => {
    const { rig } = await open({ phase: "active" });
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
    const { rig } = await open({ phase: "active" });
    await push(rig, { state: { kind: "riding" } });
    rig.world.phase = "loaded";
    await userEvent.click(screen.getByRole("button", { name: "Pause" }));
    await screen.findByRole("alert");
    expect(enabled("Pause")).toBe(true);
    expect(enabled("Resume")).toBe(false);
  });

  test("a double tap on Pause sends one pause, even when the status arrives between the taps", async () => {
    const { rig } = await open({ phase: "active" });
    await push(rig, { state: { kind: "riding" } });
    const pauseBtn = screen.getByRole("button", { name: "Pause" });
    await userEvent.click(pauseBtn);
    await waitFor(() => expect(enabled("Resume")).toBe(true));
    await userEvent.click(pauseBtn);
    expect(rig.count("ride_pause")).toBe(1);
    expect(rig.count("ride_resume")).toBe(0);
  });

  test("a double tap on Resume sends one resume, even when the status arrives between the taps", async () => {
    const { rig } = await open({ phase: "active", userPause: true });
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
    const { rig } = await open({ phase: "active", publishes: false });
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
    const { rig } = await open({
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
    const { rig } = await open({ publishes: false });
    await userEvent.click(screen.getByRole("button", { name: "Start" }));
    await waitFor(() => expect(rig.count("ride_start")).toBe(1));
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    expect(enabled("Start")).toBe(false);
    await userEvent.click(screen.getByRole("button", { name: "Start" }));
    expect(rig.count("ride_start")).toBe(1);
  });

  test("Skip waits for the plan: with it unanswered the ride cannot skip", async () => {
    let release = () => {};
    const { rig } = await open(
      { phase: "active" },
      { before: (r) => (release = r.hold("ride_plan")) },
    );
    await push(rig, { state: { kind: "riding" }, blockIndex: 0 });
    expect(enabled("Skip block")).toBe(false);
    await act(async () => release());
    await waitFor(() => expect(enabled("Skip block")).toBe(true));
  });

  // #110: Ride this, then the workout is edited or deleted in the library before the ride is opened. The screen draws
  // the ride as the core loaded it (`ride_plan`), never the library's current workout: here the library no longer
  // has it (`workout_get` is `not_found` for this id), and the name, the block and Skip still come from the ride.
  test("the screen draws the loaded ride's own workout, not the library's", async () => {
    const { rig } = await open(
      { phase: "active", workoutName: "As loaded" },
      { workoutId: "w-deleted" },
    );
    await push(rig, { state: { kind: "riding" }, blockIndex: 0 });
    expect(await screen.findByText("As loaded")).toBeTruthy();
    await waitFor(() => expect(enabled("Skip block")).toBe(true));
    expect(rig.count("ride_plan")).toBeGreaterThan(0);
    expect(rig.count("workout_get")).toBe(0);
  });

  test("a free ride has no block to skip", async () => {
    const { rig } = await open(
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
      const { rig } = await open({ phase: "active" });
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
      const { rig } = await open({ ...over, engineStopped: true });
      await push(rig, { state: { kind } as StreamStatus["state"] });
      await userEvent.click(screen.getByRole("button", { name }));
      const alert = await screen.findByRole("alert");
      expect(alert.getAttribute("data-code")).toBe("internal");
      expect(alert.textContent).toContain("engine stopped");
      expect(alert.textContent).toContain("The ride engine stopped:");
      expect(routeText()).toBe("ride:");
    },
  );

  test("a wedged engine: a command shows the core's refusal, the band says the engine is not answering, and End still leaves the ride (#351)", async () => {
    const { rig } = await open({ phase: "active" });
    await push(rig, { state: { kind: "riding" } });
    rig.wedge();
    await userEvent.click(screen.getByRole("button", { name: "Pause" }));
    const refusal = await waitFor(() => {
      const el = document.querySelector('[data-code="engine_unresponsive"]');
      expect(el).not.toBeNull();
      return el as HTMLElement;
    });
    expect(refusal.textContent).toContain("The ride engine is not answering:");
    // ride.engineUnresponsive drives the band (ADR-0001 §4.3.2) as soon as the frame carries it.
    const banner = await screen.findByTestId("engine-unresponsive");
    expect(banner.getAttribute("role")).toBe("alert");
    expect(banner.textContent).toContain("End still ends the ride");
    // One snapshot channel for the window; the frame says the engine is wedged, not a reopen.
    expect(rig.ui.count("ui_open")).toBe(1);
    await userEvent.click(screen.getByRole("button", { name: "End" }));
    await userEvent.click(screen.getByRole("button", { name: "End ride" }));
    await waitFor(() => expect(routeText()).toBe("history:ride-1"));
  });

  test("End with the engine stopped still leaves the ride: the core ends it by force (#331)", async () => {
    const { rig, client } = await open({
      phase: "active",
      engineStopped: true,
    });
    await push(rig, { state: { kind: "riding" } });
    await userEvent.click(screen.getByRole("button", { name: "End" }));
    await userEvent.click(screen.getByRole("button", { name: "End ride" }));
    await waitFor(() => expect(routeText()).toBe("history:ride-1"));
    expect(screen.queryByRole("alert")).toBeNull();
    // The forcing is in the frame's `ride.ended`, not a side set on the window (#598).
    await waitFor(() => expect(client.snapshot.ride?.ended).toBe("forced"));
  });

  test("a repeat End after a forced end still answers null (the end is in the frame's ride.ended)", async () => {
    const { rig, client } = await open({
      phase: "active",
      engineStopped: true,
    });
    await push(rig, { state: { kind: "riding" } });
    await userEvent.click(screen.getByRole("button", { name: "End" }));
    await userEvent.click(screen.getByRole("button", { name: "End ride" }));
    await waitFor(() => expect(routeText()).toBe("history:ride-1"));
    expect(client.snapshot.ride?.ended).toBe("forced");
    const again = await commands.rideEnd();
    expect(again).toEqual({ status: "ok", data: null });
  });

  test("an End the engine answered is not a forced end", async () => {
    const { rig, client } = await open({ phase: "active" });
    await push(rig, { state: { kind: "riding" } });
    await userEvent.click(screen.getByRole("button", { name: "End" }));
    await userEvent.click(screen.getByRole("button", { name: "End ride" }));
    await waitFor(() => expect(routeText()).toBe("history:ride-1"));
    await waitFor(() => expect(client.snapshot.ride?.ended).toBe("finished"));
  });

  test("Resume with the engine stopped shows the core's internal error", async () => {
    const { rig } = await open({
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

  test("the TARGET tile shows what the engine commanded, the plan is the smaller line under it (#324)", async () => {
    const { rig } = await open();
    // a soft start: the plan says 250 W, the trainer was told 120 W
    await push(rig, {
      workoutS: 100,
      blockIndex: 1,
      blockElapsedS: 20,
      commanded: { kind: "erg", watts: 120 },
    });
    const tile = slot("target")?.closest("[data-tile]");
    expect(tile?.textContent).toContain("TARGET");
    expect(slot("target")?.textContent).toBe("120");
    expect(slot("plan")?.textContent).toBe("Plan 250 W");
    expect(slot("plan")?.closest("[data-tile]")).toBe(tile);
    expect(document.body.textContent).not.toMatch(/PLAN/);
    await push(rig, {
      workoutS: 101,
      blockIndex: 1,
      blockElapsedS: 21,
      commanded: { kind: "erg", watts: 166 },
    });
    expect(slot("target")?.textContent).toBe("166");
  });

  test('the target is written like the plan, "250 W" with a space (DESIGN §18.3 "Metric grid", #581)', async () => {
    const { rig } = await open();
    await push(rig, {
      blockIndex: 1,
      blockElapsedS: 20,
      commanded: { kind: "erg", watts: 250 },
    });
    expect(slot("target")?.parentElement?.textContent).toBe(
      "TARGET 250 W · Plan 250 W",
    );
  });

  test("nothing is commanded: the tile says — and the plan stays", async () => {
    const { rig } = await open();
    await push(rig, { blockIndex: 1, blockElapsedS: 20, commanded: null });
    expect(slot("target")?.textContent).toBe("—");
    expect(slot("plan")?.textContent).toBe("Plan 250 W");
  });

  test("a release, a pause and a free block say so instead of a number", async () => {
    const { rig } = await open({ phase: "active" });
    const shown = () => slot("target")?.textContent;
    await act(async () => {
      Object.assign(rig.world, { blockIdx: 1, ergReleased: true });
      rig.publish();
    });
    expect(shown()).toBe("Released");
    expect(rig.ui.truth.ride?.status.commanded).toEqual({
      kind: "resistance",
      pct: 0,
    });
    await act(async () => {
      Object.assign(rig.world, { ergReleased: false, userPause: true });
      rig.publish();
    });
    expect(shown()).toBe("Paused");
    await act(async () => {
      Object.assign(rig.world, { userPause: false });
      rig.publish();
    });
    expect(shown()).toBe("250");
    // a free block: no watts, the engine sends the flat-road simulation
    await act(async () => {
      Object.assign(rig.world, {
        blocks: [block(0, 600, 0, 0, "free")],
        blockIdx: 0,
      });
      rig.publish();
    });
    expect(shown()).toBe("Free");
    expect(
      slot("target")?.closest("[data-tile]")?.querySelector(".tile-unit")
        ?.textContent,
    ).toBe("");
  });

  test("a rider pause outranks a release: paused and released says Paused, never Released (#330)", async () => {
    const { rig } = await open({ phase: "active", blockIdx: 1 });
    await act(async () => {
      Object.assign(rig.world, { ergReleased: true, userPause: true });
      rig.publish();
    });
    expect(rig.ui.truth.ride?.status.ergReleased).toBe(true);
    expect(rig.ui.truth.ride?.status.userPaused).toBe(true);
    expect(slot("target")?.textContent).toBe("Paused");
  });

  test("a free block is judged before a release: the mock sends Free, as the engine does", async () => {
    const { rig } = await open({
      phase: "active",
      blocks: [block(0, 600, 0, 0, "free")],
      blockIdx: 0,
      ergReleased: true,
    });
    await act(async () => rig.publish());
    expect(rig.ui.truth.ride?.status.commanded).toEqual({ kind: "free" });
    expect(slot("target")?.textContent).toBe("Free");
  });

  test('"Off" is never shown: Resume publishes the pause\'s command, then the soft start runs from 0 W (no pedalling) to the plan', async () => {
    const { rig } = await open({ phase: "active", blockIdx: 1 });
    await act(async () => rig.publish());
    await userEvent.click(screen.getByRole("button", { name: "Pause" }));
    await waitFor(() => expect(slot("target")?.textContent).toBe("Paused"));
    await userEvent.click(screen.getByRole("button", { name: "Resume" }));
    await waitFor(() =>
      expect(rig.ui.truth.ride?.status.userPaused).toBe(false),
    );
    expect(rig.ui.truth.ride?.status.commanded).toEqual({
      kind: "resistance",
      pct: 0,
    });
    expect(slot("target")?.textContent).toBe("—");
    await act(async () => rig.tick(null));
    expect(slot("target")?.textContent).toBe("0");
    await act(async () => rig.tick(null));
    expect(slot("target")?.textContent).toBe("0");
    expect(slot("plan")?.textContent).toBe("Plan 250 W");
    expect(rig.ui.truth.ride?.status.softStartTargetW).toBeNull();
    expect(screen.queryByTestId("soft-start")).toBeNull();
    const seen: (string | null | undefined)[] = [];
    await act(async () => rig.tick(150));
    seen.push(slot("target")?.textContent);
    for (let n = 0; n < 5; n++) {
      await act(async () => rig.tick(150));
      seen.push(slot("target")?.textContent);
    }
    expect(seen).toEqual(["150", "170", "190", "210", "230", "250"]);
    expect(screen.queryByTestId("soft-start")).toBeNull();
    expect(document.body.textContent).not.toMatch(/\bOff\b/);
  });

  test("Resume to a pedalling rider starts the soft start at the rider's power, not the plan", async () => {
    const { rig } = await open({
      phase: "active",
      blockIdx: 1,
      userPause: true,
    });
    await act(async () => rig.publish());
    await userEvent.click(screen.getByRole("button", { name: "Resume" }));
    await waitFor(() =>
      expect(rig.ui.truth.ride?.status.userPaused).toBe(false),
    );
    await act(async () => rig.tick(200));
    expect(slot("target")?.textContent).toBe("200");
    expect(rig.ui.truth.ride?.status.softStartTargetW).toBeNull();
    expect(screen.queryByTestId("soft-start")).toBeNull();
    await act(async () => rig.tick(200));
    expect(slot("target")?.textContent).toBe("210");
    expect(rig.ui.truth.ride?.status.softStartTargetW).toBeNull();
    expect(screen.queryByTestId("soft-start")).toBeNull();
  });

  test("pedalling stops mid-ramp: the engine sends nothing and the tile holds the last step; the ramp restarts when pedalling returns", async () => {
    const { rig } = await open({
      phase: "active",
      blockIdx: 1,
      userPause: true,
    });
    await act(async () => rig.publish());
    await userEvent.click(screen.getByRole("button", { name: "Resume" }));
    await waitFor(() =>
      expect(rig.ui.truth.ride?.status.userPaused).toBe(false),
    );
    await act(async () => rig.tick(150));
    await act(async () => rig.tick(150));
    expect(slot("target")?.textContent).toBe("170");
    await act(async () => rig.tick(null));
    expect(slot("target")?.textContent).toBe("170");
    await act(async () => rig.tick(null));
    expect(slot("target")?.textContent).toBe("170");
    await act(async () => rig.tick(100));
    expect(slot("target")?.textContent).toBe("100");
    await act(async () => rig.tick(100));
    expect(slot("target")?.textContent).toBe("130");
  });

  test("a ride without blocks commands Free, as the engine does for a free ride", async () => {
    const { rig } = await open(
      { phase: "active", blocks: [], blockIdx: 0 },
      { workoutId: null },
    );
    await act(async () => rig.publish());
    expect(rig.ui.truth.ride?.status.commanded).toEqual({ kind: "free" });
    expect(slot("target")?.textContent).toBe("Free");
  });

  test("a lost trainer commands nothing, and the tile does not keep showing the old target", async () => {
    const { rig } = await open({ phase: "active" });
    await act(async () => {
      Object.assign(rig.world, { blockIdx: 1, commandedW: 180 });
      rig.publish();
    });
    expect(slot("target")?.textContent).toBe("180");
    await act(async () => {
      Object.assign(rig.world, { trainerLost: true });
      rig.publish();
    });
    expect(rig.ui.truth.ride?.status.commanded).toBeNull();
    expect(slot("target")?.textContent).toBe("—");
  });

  test("the tile shows the engine's watts as they are: no rounding or arithmetic of its own", async () => {
    const { rig } = await open();
    await push(rig, {
      blockIndex: 1,
      blockElapsedS: 20,
      commanded: { kind: "erg", watts: 253 },
    });
    expect(slot("target")?.textContent).toBe("253");
    expect(slot("plan")?.textContent).toBe("Plan 250 W");
  });

  test("Escape closes the confirm and does not end the ride", async () => {
    const { rig } = await open({ phase: "active" });
    await push(rig, { state: { kind: "riding" } });
    await userEvent.click(screen.getByRole("button", { name: "End" }));
    expect(screen.getByRole("alertdialog")).toBeTruthy();
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(rig.count("ride_end")).toBe(0);
  });

  test("a new load replaces the ride in the frame and the screen follows it (#598)", async () => {
    const { rig, client } = await open();
    expect(rig.ui.count("ui_open")).toBe(1);
    await act(async () => rig.ui.loadStarted());
    await waitFor(() => expect(client.snapshot.loading).toBe(true));
    await act(async () => rig.ui.installed(rideView({ id: "ride-2" })));
    await waitFor(() => expect(client.snapshot.ride?.id).toBe("ride-2"));
    // The client holds one channel across loads (ADR-0001 §4.3).
    expect(rig.ui.count("ui_open")).toBe(1);
  });

  test("End asks first; Keep riding sends nothing, End ride ends and shows the summary route with the ride id", async () => {
    const { rig } = await open({ phase: "active" });
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
    const { rig, client } = await open();
    await userEvent.click(screen.getByRole("button", { name: "End" }));
    expect(screen.getByRole("alertdialog").textContent).toMatch(/not started/);
    await userEvent.click(screen.getByRole("button", { name: "End ride" }));
    await waitFor(() => expect(routeText()).toBe("workouts:"));
    expect(rig.count("ride_end")).toBe(1);
    await waitFor(() => expect(client.snapshot.ride).toBeNull());
    expect(rig.world.phase).toBe("none");
  });

  test("a refused end shows the core's reason and stays", async () => {
    const { rig } = await open({ phase: "active" });
    await push(rig, { state: { kind: "riding" } });
    rig.world.phase = "none";
    await userEvent.click(screen.getByRole("button", { name: "End" }));
    await userEvent.click(screen.getByRole("button", { name: "End ride" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("no ride loaded");
    expect(routeText()).toBe("ride:");
  });

  test("state: finished navigates to the summary route exactly once, with the ride id", async () => {
    const { rig } = await open({ phase: "active" });
    await push(rig, { state: { kind: "riding" } });
    await push(rig, { state: { kind: "finished" } });
    await waitFor(() => expect(routeText()).toBe("history:ride-1"));
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
    const { rig } = await open({ phase: "active" });
    await push(rig, { state: { kind: "finished" } });
    expect(
      (screen.getByRole("button", { name: "End" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
  });

  test("the device pill follows the links, with a word besides the colour", async () => {
    const { rig } = await open();
    await act(async () => {
      rig.world.hrLost = true;
      rig.publish();
    });
    const pill = screen.getByLabelText("Devices");
    expect(pill.textContent).toContain("NEO");
    expect(pill.textContent).toContain("ready");
    expect(pill.textContent).toContain("⚠ HR");
    expect(pill.textContent).toContain("lost");
  });

  test("the chart draws the target profile and the cursor advances with the workout clock, and the traces draw ui.series points as the channel appends them", async () => {
    const { rig } = await open({ phase: "active" });
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
    // Points come through ui.series: the channel appends; the trace redraws (ADR-0001 §4.3).
    await act(async () => {
      rig.ui.setStatus({ workoutS: 301 });
      rig.ui.append("power", 5, 5, 240);
    });
    await act(async () => {
      rig.ui.setStatus({ workoutS: 302 });
      rig.ui.append("power", 6, 6, 250);
    });
    expect(
      screen.getByTestId("chart-power").getAttribute("points")?.split(" "),
    ).toHaveLength(2);
  });

  // The old "a sample delivered twice is one point of the history" tested session.ts's dedup, which issue #598
  // removed: the snapshot channel is additive (the core sends each point once from the cursor, stream/ui.test.tsx's
  // "appends the series…" covers it). Deleted.

  test("no layout shift: a slot's size is set by its widest form, not by the value", async () => {
    const { rig } = await open();
    const sizes = () =>
      [
        "power",
        "target",
        "plan",
        "hr",
        "cadence",
        "countdown",
        "elapsed",
        "left",
      ].map((id) => (slot(id) as HTMLElement | null)?.style.width ?? "");
    await push(
      rig,
      { workoutS: 5, blockElapsedS: 1 },
      { powerW: 5, hrBpm: 61 },
    );
    const short = sizes();
    await push(
      rig,
      { workoutS: 3599, blockElapsedS: 99 },
      { powerW: 1234, hrBpm: 188 },
    );
    expect(slot("power")?.textContent).toBe("1234");
    expect(sizes()).toEqual(short);
    expect(short.every((w) => /^\d+ch$/.test(w) || w === "")).toBe(true);
    expect(short.slice(0, 5).every((w) => /^\d+ch$/.test(w))).toBe(true);
    expect(short[6]).toBe("7ch");
  });

  test("the band says the engine is not answering when the channel is silent for 2 s, and recovers (ADR-0001 §4.3.4)", async () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    const fake = new FakeClock();
    const { rig } = await open({ phase: "active" }, { clock: fake });
    expect(screen.queryByTestId("engine-unresponsive")).toBeNull();
    rig.ui.stall();
    await fake.advance(WATCHDOG_MS);
    await waitFor(() =>
      expect(screen.getByTestId("engine-unresponsive")).toBeTruthy(),
    );
    rig.ui.resume();
    await waitFor(() =>
      expect(screen.queryByTestId("engine-unresponsive")).toBeNull(),
    );
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
    const { rig } = await open({ phase: "active" });
    await push(
      rig,
      { workoutS: 100, blockIndex: 1, blockElapsedS: 65 },
      samples ? { powerW: 240, cadenceRpm: 90, hrBpm: 150 } : undefined,
    );
    return rig;
  };

  test("trainer lost: the banner names the pause point, power and cadence say —, HR stays", async () => {
    const rig = await rideWith();
    await publish(rig, { trainerLost: true, blockIdx: 1 });
    const banner = screen.getByTestId("loss-trainer");
    expect(banner.textContent).toContain("NEO lost");
    expect(screen.getByTestId("caption").textContent).toMatch(/^paused at /);
    expect(banner.textContent).toContain("keep pedalling or stop");
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
    const { rig } = await open({ phase: "active" });
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
    await publish(rig, { stoppedPedalling: true });
    expect(rig.ui.truth.ride?.status.ergReleased).toBe(true);
    expect(rig.ui.truth.ride?.status.state.kind).toBe("paused");
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
    await push(
      rig,
      { state: { kind: "riding" } },
      { powerW: null, cadenceRpm: null },
    );
    expect(slot("power")?.textContent).toBe("—");
    expect(screen.queryByTestId("loss-trainer")).toBeNull();
    expect(screen.queryByTestId("loss-hr")).toBeNull();
  });

  test("no loss rows without a live ride: a lost link before Start or after the end is not a loss", async () => {
    const { rig } = await open({ phase: "loaded" });
    await act(async () => {
      rig.world.hrLost = true;
      rig.publish();
    });
    expect(screen.queryByTestId("loss-hr")).toBeNull();
  });
});

describe("End stays usable (#327)", () => {
  test("a ride command that never answers does not hold End", async () => {
    const { rig } = await open({ phase: "active" });
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
    const { rig } = await open({ phase: "active", publishes: false });
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
    const { rig } = await open({ phase: "active" });
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
    const { rig } = await open({ phase: "active", userPause: true });
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
    const { rig } = await open({ phase: "active" });
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
    await publish(rig, { trainerLost: false, trainerSilent: true });
    expect(text("loss-trainer")).toContain("No data from the NEO");
    expect(text("loss-trainer")).not.toContain("reconnecting");
    expect(text("loss-trainer")).not.toContain("attempt");
  });

  test("the key row says nothing of the trainer's mode or following while it is lost: `commanded` is stale then", async () => {
    const { rig } = await open({ phase: "active" });
    const keyRow = () =>
      document.querySelector(".ride-status")?.textContent ?? "";
    await publish(rig, { blockIdx: 1 });
    expect(keyRow()).toContain("trainer in ERG at 250 W, following");
    await publish(rig, { trainerSilent: true, sessionS: 100, lostSinceS: 100 });
    expect(rig.ui.truth.ride?.status.commanded).toEqual({
      kind: "erg",
      watts: 0,
    });
    expect(keyRow()).not.toMatch(/trainer in|following/);
    await publish(rig, {
      trainerSilent: false,
      sessionS: 101,
      gapS: 1,
      reconnectedAtS: 101,
    });
    expect(keyRow()).toContain("following");
  });

  test("lost for 29 s: no what-to-try row; at 30 s on the session clock it adds the text and End…", async () => {
    const rig = await lostRide();
    await publish(rig, { sessionS: 129 });
    expect(screen.queryByTestId("loss-help")).toBeNull();
    expect(screen.queryByRole("button", { name: "End…" })).toBeNull();
    await publish(rig, { sessionS: 130 });
    expect(text("loss-help")).toContain("power-cycle · close Zwift");
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
      const { rig } = await open({ phase: "active" });
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
    const { rig } = await open({ phase: "active" });
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
    const { rig } = await open({ phase: "active" });
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
    const { rig } = await open({ phase: "active" });
    await publish(rig, { pausedAfterLoss: true, gapS: 190, reconnectedAtS: 5 });
    expect(text("re-entry")).toContain("↻ Back after 3:10");
    await publish(rig, { pausedAfterLoss: false });
    expect(screen.queryByTestId("re-entry")).toBeNull();
  });

  test("reconnected soft start: 'easing back in to <W> W' while the core reports a target", async () => {
    const { rig } = await open({ phase: "active" });
    await publish(rig, { gapS: 14, reconnectedAtS: 200, sessionS: 202 });
    expect(screen.queryByTestId("soft-start")).toBeNull();
    await publish(rig, { softStartW: 253 });
    expect(text("soft-start")).toBe("Reconnected · easing back in to 253 W");
    await publish(rig, { softStartW: null, sessionS: 203 });
    expect(screen.queryByTestId("soft-start")).toBeNull();
  });

  test("the 'NEO back · m:ss gap' chip lives 10 s on the session clock", async () => {
    const { rig } = await open({ phase: "active" });
    await publish(rig, { gapS: 14, reconnectedAtS: 200, sessionS: 200 });
    expect(text("neo-back")).toBe("NEO back · 0:14 gap");
    await publish(rig, { sessionS: 209 });
    expect(text("neo-back")).toBe("NEO back · 0:14 gap");
    await publish(rig, { sessionS: 210 });
    expect(text("neo-back")).toBe("");
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
    expect(rig.ui.truth.ride?.status.state).toEqual({
      kind: "paused",
      reason: "trainer_lost",
    });
    expect(rig.ui.truth.ride?.status.userPaused).toBe(true);
    expect(enabled("Resume")).toBe(true);
    expect(enabled("Pause")).toBe(false);
    expect(
      screen.getByRole("button", { name: "Pause" }).getAttribute("title"),
    ).toBe("Already paused");
    await userEvent.click(screen.getByRole("button", { name: "Resume" }));
    await waitFor(() => expect(rig.count("ride_resume")).toBe(1));
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
    const { rig } = await open({ phase: "active" });
    // `trainer_lost` with every loss field null (a shape the core makes only when no loss is reported): nothing to show.
    await act(async () => {
      rig.ui.truth.ride!.status = makeStatus({
        state: { kind: "paused", reason: "trainer_lost" },
        sessionS: 900,
      });
      rig.ui.changed();
    });
    for (const id of ["loss-trainer", "loss-help", "re-entry", "soft-start"])
      expect(screen.queryByTestId(id)).toBeNull();
    expect(text("neo-back")).toBe("");
    expect(text("state")).toBe("Paused: waiting for you to pedal");
    await push(rig, { state: { kind: "riding" }, sessionS: 901 });
    expect(text("neo-back")).toBe("");
    expect(screen.queryByTestId("re-entry")).toBeNull();
  });

  test("never inferred: a lost stamp with no session clock gives no duration, so no ≥ 30 s row", async () => {
    const { rig } = await open({ phase: "active" });
    await push(rig, {
      state: { kind: "paused", reason: "trainer_lost" },
      trainerLostSinceS: 1,
      sessionS: null,
    });
    expect(screen.getByTestId("loss-trainer")).toBeTruthy();
    expect(screen.queryByTestId("loss-help")).toBeNull();
  });
});

describe("the heart-rate zone (DESIGN §18.3, #586), from the loaded ride's snapshot (ADR-0009 §2.1.4)", () => {
  const measured = (value: number) =>
    ({ value, sd: 5, source: "self_report", estimateId: 1 }) as const;
  /** The snapshot frozen at the load: `PRIOR_SNAPSHOT` with these HR references. */
  const at = (lthr: number | null, hrMax: number | null) => ({
    athlete: {
      ...PRIOR_SNAPSHOT,
      lthr: lthr === null ? null : measured(lthr),
      hrMax: hrMax === null ? null : measured(hrMax),
    },
  });
  const riding = async (hr: number | null, world: Partial<RideWorld> = {}) => {
    const { rig } = await open({ phase: "active", ...world });
    await waitFor(() => expect(rig.count("ride_plan")).toBeGreaterThan(0));
    await push(
      rig,
      { workoutS: 100, blockIndex: 1, blockElapsedS: 65 },
      { hrBpm: hr },
    );
    return rig;
  };
  const sub = () =>
    slot("hr")?.closest(".readout")?.querySelector(".readout-sub");
  afterEach(forgetAthleteModel);

  test("with an LTHR the HR tile names Friel's zone of the value it shows", async () => {
    const rig = await riding(141.6, at(165, 190));
    await waitFor(() => expect(slot("hr-zone")?.textContent).toBe("Z2 · LTHR"));
    expect(slot("hr")?.textContent).toBe("142");
    await push(
      rig,
      { workoutS: 101, blockIndex: 1, blockElapsedS: 66 },
      { hrBpm: 166 },
    );
    expect(slot("hr-zone")?.textContent).toBe("Z5a · LTHR");
  });

  test("with only an HR max it names the five-zone zone", async () => {
    await riding(142, at(null, 190));
    await waitFor(() =>
      expect(slot("hr-zone")?.textContent).toBe("Z3 · HRmax"),
    );
  });

  test("a free ride reads its snapshot too", async () => {
    await riding(142, { ...at(null, 190), workoutId: null });
    await waitFor(() =>
      expect(slot("hr-zone")?.textContent).toBe("Z3 · HRmax"),
    );
  });

  test("a prior HR max is no zone reference: the tile shows no zone", async () => {
    await riding(142, {
      athlete: {
        ...PRIOR_SNAPSHOT,
        hrMax: { value: 180, sd: 10, source: "prior", estimateId: null },
      },
    });
    expect(slot("hr")?.textContent).toBe("142");
    expect(slot("hr-zone")).toBeNull();
    expect(sub()?.textContent).toBe("");
  });

  test("with neither reference the tile shows no zone", async () => {
    await riding(142);
    expect(slot("hr")?.textContent).toBe("142");
    expect(slot("hr-zone")).toBeNull();
    expect(sub()?.textContent).toBe("");
  });

  test("Settings is not read: an LTHR typed in before or during the ride does not move the live zone", async () => {
    // The model has an LTHR of 140 now (typed in after this ride was loaded with only an HR max of 190).
    const model = installAthlete({ rows: [estimate(1, "lthr", 140)] });
    await refreshAthleteModel();
    const rig = await riding(150, at(null, 190));
    await waitFor(() =>
      expect(slot("hr-zone")?.textContent).toBe("Z3 · HRmax"),
    );
    // Mid-ride the rider types another one; the ride keeps its snapshot.
    await act(async () => {
      await commands.athleteSet("lthr", 150, null);
      await refreshAthleteModel();
    });
    await push(
      rig,
      { workoutS: 101, blockIndex: 1, blockElapsedS: 66 },
      { hrBpm: 150 },
    );
    expect(slot("hr-zone")?.textContent).toBe("Z3 · HRmax");
    expect(model.calls.filter((c) => c.cmd === "athlete_model")).toHaveLength(
      2,
    );
  });

  test("no HR yet, or the strap lost: no zone; the loss says why", async () => {
    const rig = await riding(null, at(165, null));
    expect(slot("hr-zone")).toBeNull();
    await act(async () => {
      Object.assign(rig.world, { hrLost: true });
      rig.publish();
    });
    expect(slot("hr-zone")).toBeNull();
    expect(sub()?.textContent).toBe("HR strap lost");
  });
});
