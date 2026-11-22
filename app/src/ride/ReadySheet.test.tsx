// @vitest-environment jsdom
// The Ready sheet (DESIGN §5.4, #526, #598): its state from the snapshot frame's `loading` and `devices`, and the ride
// screen before Start showing it.
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
import { BLOCKS, HRM, NEO, installRide, type RideWorld } from "./rideBackend";
import { resetLeft } from "./session";
import { resetRideLoad, startRideLoad } from "../views/rideLoad";
import { compiled, install as installWorkouts } from "../views/workoutsBackend";
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
    readyView({ loading: false, devices: BOTH_READY, refused: null, ...over });

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

  test("while a load has not installed its ride each row shows its device's keeper phase and Start waits (#681)", () => {
    // ADR-0002 §5.1.4 "Load phases": "Trainer: searching… / connecting… / ready", "HR: not found". Before an attempt
    // starts (`cold`) the row is still checking.
    const loading = (trainer: LinkPhase, strap: LinkPhase) =>
      view({ loading: true, devices: devices({}, { trainer, strap }) });
    expect(loading("cold", "cold")).toMatchObject({
      state: "loading",
      trainer: "checking",
      hr: "checking",
      canStart: false,
      line: "Start is enabled once the ride has loaded",
    });
    expect(loading("searching", "connecting")).toMatchObject({
      state: "loading",
      trainer: "searching",
      hr: "connecting",
      canStart: false,
    });
    expect(loading("handshake", "not_found")).toMatchObject({
      trainer: "connecting",
      hr: "lost",
      canStart: false,
    });
    expect(loading("warm", "warm")).toMatchObject({
      trainer: "connected",
      hr: "connected",
      canStart: false,
    });
    // From the hand-over until the ride is installed the ride holds them, in the state they were handed over in.
    expect(view({ loading: true, devices: BOTH_READY })).toMatchObject({
      state: "loading",
      trainer: "connected",
      hr: "connected",
      canStart: false,
    });
    expect(readyBanner(view({ loading: true })).title.text).toBe(
      "Loading the ride…",
    );
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

  test("most urgent first: a refusal, then loading, then the trainer, then the strap", () => {
    expect(view({ refused: "start", devices: BOTH_LOST }).state).toBe("error");
    expect(view({ loading: true, devices: BOTH_LOST }).state).toBe("loading");
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
      view({ loading: true }),
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
  resetRideLoad();
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

/**
 * A load of w1 under way whose answer never comes: the frame says `loading` and has no ride (ui.rs `Ui::load_started`).
 * `own`: this window asked for it (`startRideLoad`); otherwise another window did, and this one does not know the workout.
 */
async function openLoading(own: boolean) {
  const library = installWorkouts({
    summaries: [
      {
        id: "w1",
        name: "Steady 10 min",
        totalS: 600,
        tss: 10,
        ftp: 250,
        builtin: true,
      },
    ],
    compiled: { w1: compiled({ blocks: BLOCKS, totalS: 600 }) },
    gate: new Promise<void>(() => {}),
  });
  const rig = installRide({ phase: "none", blocks: BLOCKS });
  library.ui = rig.ui;
  if (own) void startRideLoad("w1", "Steady 10 min");
  else rig.ui.loadStarted();
  const client = new UiClient(neverClock);
  render(
    <UiProvider client={client}>
      <Ride />
    </UiProvider>,
  );
  await waitFor(() => expect(client.snapshot.loading).toBe(true));
  return { rig, client };
}

describe("the ride screen while a load has not installed its ride", () => {
  test("the first frame's `loading` draws the ride layout: rows checking, the plan, empty tiles, Start waiting", async () => {
    const { rig } = await openLoading(true);
    expect(rig.ui.truth.ride).toBeNull();
    await waitFor(() => expect(sheet().dataset.ready).toBe("loading"));
    expect(band()).toContain("Loading the ride…");
    const rows = within(sheet()).getAllByRole("listitem");
    expect(rows.map((r) => r.dataset.state)).toEqual(["checking", "checking"]);
    expect(within(sheet()).getAllByText(/↻ checking…/)).toHaveLength(2);
    // The plan of the workout this window asked to load.
    await waitFor(() =>
      expect(screen.getByTestId("ready-plan").textContent).toBe(
        "ERG · starts with 3:00 warm-up",
      ),
    );
    expect(document.querySelector(".ride-title")?.textContent).toBe(
      "Steady 10 min",
    );
    expect(
      document.querySelector("[data-region='timeline']")?.textContent,
    ).toContain("Work 250 W");
    // Nothing is the ride's yet: the tiles are empty, the pills not connected, the countdown waits for the ride.
    for (const id of ["power", "hr", "cadence"])
      expect(document.querySelector(`[data-slot='${id}']`)?.textContent).toBe(
        "—",
      );
    expect(document.querySelector("[data-slot='block']")?.textContent).toBe(
      "Loading the plan…",
    );
    expect(
      [...document.querySelectorAll(".pill-half")].map(
        (p) => (p as HTMLElement).dataset.link,
      ),
    ).toEqual(["none", "none"]);
    expect(keyLine()).toBe("Start is enabled once the ride has loaded");
    expect(start().disabled).toBe(true);
    const end = screen.getByRole("button", {
      name: "End",
    }) as HTMLButtonElement;
    expect(end.disabled).toBe(true);
    // Never the bare page once a frame has come.
    expect(
      screen.queryByRole("heading", { name: "Loading the ride…" }),
    ).toBeNull();
  });

  test("the rows follow the keeper's phases frame by frame as the load connects (#681)", async () => {
    const { rig } = await openLoading(true);
    await waitFor(() => expect(sheet().dataset.ready).toBe("loading"));
    const rows = () =>
      within(sheet())
        .getAllByRole("listitem")
        .map((r) => [
          r.dataset.state,
          r.querySelector("[data-state-label]")?.textContent,
        ]);
    const walk: [LinkPhase, LinkPhase, [string, string][]][] = [
      [
        "searching",
        "searching",
        [
          ["searching", "↻ searching…"],
          ["searching", "↻ searching…"],
        ],
      ],
      [
        "handshake",
        "not_found",
        [
          ["connecting", "↻ connecting…"],
          ["lost", "⚠ not found"],
        ],
      ],
      [
        "warm",
        "not_found",
        [
          ["connected", "✓ connected"],
          ["lost", "⚠ not found"],
        ],
      ],
    ];
    for (const [trainer, strap, want] of walk) {
      act(() =>
        rig.setKeeper({
          trainer: { phase: trainer, sample: null },
          strap: { phase: strap, sample: null },
        }),
      );
      await waitFor(() => expect(rows()).toEqual(want));
      expect(band()).toContain("Loading the ride…");
      expect(start().disabled).toBe(true);
    }
  });

  test("the installed ride replaces the loading state in the same layout", async () => {
    const { rig } = await openLoading(false);
    await waitFor(() => expect(sheet().dataset.ready).toBe("loading"));
    // ui.rs `Ui::install`, then `Ui::load_over`: the next frame carries the ride and no `loading`.
    await act(async () => {
      rig.world.phase = "loaded";
      rig.ui.truth.loading = false;
      rig.publish();
    });
    await waitFor(() => expect(sheet().dataset.ready).toBe("ready"));
    expect(screen.getByTestId("ready-plan").textContent).toBe(
      "ERG · starts with 3:00 warm-up",
    );
  });

  test("a load another window started: the same layout, the plan not known", async () => {
    await openLoading(false);
    await waitFor(() => expect(sheet().dataset.ready).toBe("loading"));
    expect(screen.getByTestId("ready-plan").textContent).toBe(
      "Loading the plan…",
    );
    expect(document.querySelector(".ride-title")?.textContent).toBe("");
    expect(start().disabled).toBe(true);
  });
});

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
