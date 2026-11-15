// @vitest-environment jsdom
// The Ready sheet (DESIGN §5.4, #526, #598): its state from the snapshot frame's `devices`, and the ride screen
// before Start showing it.
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
import { afterEach, beforeEach, describe, expect, test } from "vitest";
import type { DevicesView, LinkPhase, LinkStatus } from "../bindings";
import { navigate } from "../router";
import {
  readyBanner,
  readyView,
  type ReadyState,
  type RideCommand,
} from "./ReadySheet";
import { LINE_MAX } from "./banners";
import { Ride } from "./Ride";
import { HRM, NEO, installRide, type RideWorld } from "./rideBackend";
import { resetLeft } from "./session";
import { UiClient, type Timers } from "../stream/ui";
import { UiProvider } from "../stream/uiHooks";
import { devicesOf } from "../stream/uiBackend";

const neverClock: Timers = {
  setTimeout: () => 0,
  clearTimeout: () => {},
};

const devices = (
  links: {
    trainer?: LinkStatus | null;
    strap?: LinkStatus | null;
  } = {},
  phases: { trainer?: LinkPhase; strap?: LinkPhase } = {},
): DevicesView => devicesOf({ trainer: NEO, strap: HRM }, links, phases);

/** Both devices connected (ride holds them with link "ready"). */
const BOTH_READY = devices({ trainer: "ready", strap: "ready" });
/** The strap the ride does not hold (phase `not_found`, link null); the ride holds the trainer. */
const STRAP_NOT_HELD = devices({ trainer: "ready" }, { strap: "not_found" });
const TRAINER_CONNECTING = devices({
  trainer: "connecting",
  strap: "ready",
});
const TRAINER_LOST = devices({ trainer: "lost", strap: "ready" });
const BOTH_LOST = devices({ trainer: "lost", strap: "lost" });

describe("readyView", () => {
  const view = (over: Partial<Parameters<typeof readyView>[0]> = {}) =>
    readyView({ devices: BOTH_READY, refused: null, ...over });

  test("both devices on ready links are connected and the sheet is ready", () => {
    expect(view()).toMatchObject({
      state: "ready",
      trainer: "connected",
      hr: "connected",
      refused: null,
      canStart: true,
    });
  });

  test("the strap row follows devices.strap (#598): a strap the ride does not hold is strap-missing", () => {
    expect(view({ devices: STRAP_NOT_HELD })).toMatchObject({
      state: "strap-missing",
      hr: "lost",
      canStart: true,
    });
    expect(view({ devices: BOTH_READY })).toMatchObject({
      state: "ready",
      hr: "connected",
    });
  });

  test("a trainer link that is not ready holds the start (§5.4)", () => {
    expect(view({ devices: TRAINER_CONNECTING })).toMatchObject({
      state: "connecting",
      canStart: false,
    });
    expect(view({ devices: TRAINER_LOST })).toMatchObject({
      state: "trainer-missing",
      trainer: "lost",
      canStart: false,
    });
  });

  test("most urgent first: a refusal, then the trainer, then the strap", () => {
    expect(view({ refused: "start", devices: BOTH_LOST }).state).toBe("error");
    expect(view({ devices: BOTH_LOST }).state).toBe("trainer-missing");
    const connectingLost = devices({
      trainer: "connecting",
      strap: "lost",
    });
    expect(view({ devices: connectingLost }).state).toBe("connecting");
  });

  test("a refusal is titled by the command the core refused, and the line keeps the devices' state", () => {
    const title = (refused: RideCommand) =>
      readyBanner(view({ refused })).title.text;
    expect(title("start")).toBe("Could not start");
    expect(title("end")).toBe("Could not unload the ride");
    const lost = view({ refused: "end", devices: TRAINER_LOST });
    expect(lost.state).toBe("error");
    expect(lost.canStart).toBe(false);
    expect(lost.line).toBe(view({ devices: TRAINER_LOST }).line);
  });

  test("the key row's line follows the devices and never repeats the band", () => {
    const cases: [DevicesView, ReadyState, string][] = [
      [
        TRAINER_CONNECTING,
        "connecting",
        "Start is enabled once the NEO is connected",
      ],
      [TRAINER_LOST, "trainer-missing", "Start needs the NEO connected"],
      [
        STRAP_NOT_HELD,
        "strap-missing",
        "Press Start; the trainer waits for you to pedal",
      ],
      [BOTH_READY, "ready", "Press Start; the trainer waits for you to pedal"],
    ];
    for (const [d, state, line] of cases) {
      const v = view({ devices: d });
      expect(v.state).toBe(state);
      expect(v.line).toBe(line);
      const b = readyBanner(v);
      for (const l of [b.title, ...b.actions, ...b.status])
        expect(line.toLowerCase()).not.toContain(l.text.toLowerCase());
    }
  });

  test("every banner line fits the band's 26 characters (§6.1)", () => {
    const views = [
      view({ refused: "start" }),
      view({ refused: "end" }),
      view({ devices: TRAINER_LOST }),
      view({ devices: TRAINER_CONNECTING }),
      view({ devices: STRAP_NOT_HELD }),
      view(),
    ];
    for (const v of views) {
      const b = readyBanner(v);
      for (const l of [b.title, ...b.actions, ...b.status])
        expect(l.text.length, `${v.state}: ${l.text}`).toBeLessThanOrEqual(
          LINE_MAX,
        );
      if (b.compact !== undefined)
        expect(b.compact.length).toBeLessThanOrEqual(LINE_MAX);
    }
  });
});

beforeEach(() => navigate("ride"));
afterEach(() => {
  resetLeft();
  cleanup();
  clearMocks();
});

type Rig = ReturnType<typeof installRide>;
async function open(
  over: Partial<RideWorld> = {},
  opts: {
    workoutId?: string | null;
    before?: (rig: Rig) => void;
  } = {},
) {
  const world: Partial<RideWorld> = { ...over };
  if (opts.workoutId !== undefined) world.workoutId = opts.workoutId;
  const rig = installRide(world);
  opts.before?.(rig);
  const client = new UiClient(neverClock);
  render(
    <UiProvider client={client}>
      <Ride />
    </UiProvider>,
  );
  await waitFor(() => expect(rig.ui.count("ui_open")).toBe(1));
  await waitFor(() => expect(client.snapshot.received).toBe(true));
  return { rig, client };
}

const sheet = () => screen.getByRole("region", { name: "Ready check" });
const band = () =>
  document.querySelector("[data-region='banner']")?.textContent ?? "";
/** The key row's state line, whole: before Start nothing follows it (no trainer mode, no "following"). */
const keyLine = () => document.querySelector(".ride-status")?.textContent ?? "";
const start = () =>
  screen.getByRole("button", { name: "Start" }) as HTMLButtonElement;

describe("the ride screen before Start is the Ready sheet", () => {
  test("ready: both devices connected, the plan's first block, Start enabled and focused", async () => {
    await open();
    await waitFor(() => expect(sheet().dataset.ready).toBe("ready"));
    expect(band()).toContain("Ready to ride");
    expect(keyLine()).toBe("Press Start; the trainer waits for you to pedal");
    const rows = within(sheet()).getAllByRole("listitem");
    expect(rows.map((r) => r.dataset.state)).toEqual([
      "connected",
      "connected",
    ]);
    await waitFor(() =>
      expect(screen.getByTestId("ready-plan").textContent).toBe(
        "ERG · starts with 3:00 warm-up",
      ),
    );
    // The traces come with the start: before it the column is the sheet's.
    expect(document.querySelector(".trace-stack")).toBeNull();
    expect(start().disabled).toBe(false);
    await waitFor(() => expect(document.activeElement).toBe(start()));
  });

  test("connecting: a trainer link coming up disables Start and says why", async () => {
    const { rig } = await open({ trainer: "connecting" });
    expect(sheet().dataset.ready).toBe("connecting");
    expect(band()).toContain("Connecting the NEO…");
    expect(keyLine()).toBe("Start is enabled once the NEO is connected");
    expect(start().disabled).toBe(true);
    await act(async () => {
      rig.world.trainer = "ready";
      rig.publish();
    });
    await waitFor(() => expect(sheet().dataset.ready).toBe("ready"));
    expect(start().disabled).toBe(false);
  });

  test("trainer missing: a lost trainer link disables Start, with what to try", async () => {
    const { rig } = await open({ trainer: "lost" });
    expect(sheet().dataset.ready).toBe("trainer-missing");
    expect(band()).toContain("NEO not found");
    expect(band()).toContain("close apps using the NEO");
    expect(
      sheet().querySelector("[data-device='trainer']")?.textContent,
    ).toContain("⚠ not found");
    // Never "the trainer waits for you" or "following" beside a trainer that is not there.
    expect(keyLine()).toBe("Start needs the NEO connected");
    expect(start().disabled).toBe(true);
    await userEvent.click(start());
    expect(rig.count("ride_start")).toBe(0);
  });

  test("strap missing: the ride does not hold the strap; Start stays enabled and the consequence is stated", async () => {
    const { rig } = await open({ strapHanded: false });
    await waitFor(() => expect(sheet().dataset.ready).toBe("strap-missing"));
    expect(band()).toContain("HR not found");
    expect(band()).toContain("this ride runs without HR");
    expect(keyLine()).toBe("Press Start; the trainer waits for you to pedal");
    expect(start().disabled).toBe(false);
    await userEvent.click(start());
    await waitFor(() => expect(rig.world.phase).toBe("active"));
  });

  test("error: a refused Start shows the core's reason in the sheet, and the next try clears it", async () => {
    const { rig } = await open({ recordFails: true });
    await userEvent.click(start());
    const alert = await within(sheet()).findByRole("alert");
    expect(alert.textContent).toBe(
      "The ride could not be recorded: cannot open the record",
    );
    expect(sheet().dataset.ready).toBe("error");
    expect(band()).toContain("Could not start");
    // The reason is T3, like the rest of the body (§5.4).
    expect(alert.closest(".ready-refusal")).not.toBeNull();
    expect(keyLine()).toBe("Press Start; the trainer waits for you to pedal");
    rig.world.recordFails = false;
    await userEvent.click(start());
    await waitFor(() => expect(rig.world.phase).toBe("active"));
    expect(screen.queryByRole("alert")).toBeNull();
  });

  test("error: a refused End (unloading the unstarted ride) is not called a failed start", async () => {
    const { rig } = await open();
    // The core no longer holds the ride: `ride_end` answers "no ride loaded" (core.rs `Core::end_ride`).
    rig.world.phase = "none";
    await userEvent.click(screen.getByRole("button", { name: "End" }));
    await userEvent.click(screen.getByRole("button", { name: "End ride" }));
    const alert = await within(sheet()).findByRole("alert");
    expect(alert.textContent).toContain("no ride loaded");
    expect(sheet().dataset.ready).toBe("error");
    expect(band()).toContain("Could not unload the ride");
    expect(band()).not.toContain("Could not start");
  });

  test("a free ride says so instead of a plan", async () => {
    await open({ blocks: [] }, { workoutId: null });
    expect(screen.getByTestId("ready-plan").textContent).toBe(
      "Free ride · no targets",
    );
  });

  test("the sheet leaves with the start: the traces and the ride's key row take its place", async () => {
    const { rig } = await open();
    await userEvent.click(start());
    await waitFor(() => expect(rig.world.phase).toBe("active"));
    await waitFor(() =>
      expect(screen.queryByRole("region", { name: "Ready check" })).toBeNull(),
    );
    expect(screen.getByRole("button", { name: "Pause" })).toBeTruthy();
    expect(band()).not.toContain("Ready to ride");
  });
});
