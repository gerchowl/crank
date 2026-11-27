// The design fixture entry (#457): mounts the real `AppRoot` on the repo's mockIPC backends in the state that
// `?scenario=<name>` names (scenarios.ts). Dev and test only: it has its own Vite root (vite.config.ts here), so the
// production build (app/index.html) never sees it. Each screen installs only the backend it calls, so every refusal
// is the backend's own (their headers cite the core); a command no backend answers is Tauri's plain-string rejection.
// Every screen reads the live state from the window's snapshot channel (ADR-0001 §4.3), so every scenario has one
// (stream/uiBackend.ts) that sends the core's 1 s heartbeat: without it the channel's watchdog would mark the state stale
// after 2 s. `window.__fixture.settled()` resolves once no mock answer is in flight (bar the scenario's `held` ones).
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import { AppRoot } from "../../src/App";
import { markSeen } from "../../src/build";
import type { WaitForCore } from "../../src/core";
import "../../src/theme.css";
import "../../src/shell.css";
import type {
  LinkPhase,
  LinkStatus,
  LiveValues,
  PairedDevice,
  RideView,
} from "../../src/bindings";
import { navigate } from "../../src/router";
import { setAthleteHr } from "../../src/athleteHr";
import {
  BLOCKS,
  block,
  installRide,
  type RideWorld,
} from "../../src/ride/rideBackend";
import {
  installDevices,
  strap,
  trainer,
  type DevicesState,
} from "../../src/views/devicesBackend";
import {
  RIDES,
  install as installHistory,
  type Backend as HistoryBackend,
} from "../../src/views/historyBackend";
import type { BlockSummary, RideRecord } from "../../src/bindings";
import {
  ERROR_ISSUE,
  SUMMARIES,
  WARNING,
  compiled,
  install as installWorkouts,
  type Backend as WorkoutsBackend,
} from "../../src/views/workoutsBackend";
import { install as installToday } from "../../src/views/todayBackend";
import {
  devicesOf,
  installUi,
  rideView,
  type UiBackend,
  type UiOptions,
} from "../../src/stream/uiBackend";
import { status as makeStatus } from "../../src/stream/testBackend";
import { startRideLoad } from "../../src/views/rideLoad";
import { SCENARIOS, type ScenarioName } from "./scenarios";
import { windowClose } from "../../src/editor/windowClose";

// The page has no Tauri window here: the editor's close-request listener (windowClose.ts) is held by nobody.
windowClose.listen = () => Promise.resolve(() => {});
windowClose.destroy = () => Promise.resolve();

/** Mock answers (and fixture frames) in flight; see `settled()`. */
let pending = 0;

/**
 * The snapshot channel for scenarios whose backends do not install one (workouts, history, settings): nothing
 * paired, no ride. Installed once, before the scenario's own backends, which pass its commands on; a backend that
 * installs its own channel (the ride's, the devices', Today's) answers them itself.
 */
let baseUi: UiBackend | null = null;
const base = (): UiBackend => {
  if (baseUi === null) throw new Error("the base channel is installed first");
  return baseUi;
};

type Invoke = (cmd: string, args?: unknown) => Promise<unknown>;
const internals = () =>
  (window as unknown as { __TAURI_INTERNALS__: { invoke: Invoke } })
    .__TAURI_INTERNALS__;

const neo: PairedDevice = {
  role: "trainer",
  id: trainer.id,
  name: trainer.name,
  serial: "55624",
  model: "Neo",
};
const hrm: PairedDevice = {
  role: "heart_rate",
  id: strap.id,
  name: strap.name,
  serial: null,
  model: null,
};

/** The core's heartbeat (ui.rs `HEARTBEAT`): a frame at least every second, so the watchdog never fires. */
const BEAT: UiOptions = { heartbeatMs: 1000 };

const devices = (over: Partial<DevicesState>) => () => {
  installDevices({ nearby: [trainer, strap], ...over }, BEAT);
  navigate("settings", "devices");
};

/** This tick's values (`ride.live`). */
const RIDING: LiveValues = { powerW: 248, cadenceRpm: 89, hrBpm: 142 };
/** Nothing live: before the start. */
const NO_LIVE: LiveValues = { powerW: null, cadenceRpm: null, hrBpm: null };

/**
 * The traces' history up to `until` session seconds, 1 Hz, from fixed formulas (no randomness): power near the plan of
 * rideBackend `BLOCKS`, heart rate rising with it, cadence around 90.
 */
const HISTORY = (until: number) =>
  Array.from({ length: until }, (_, t) => {
    const plan = t < 180 ? 100 + (80 * t) / 180 : 250;
    return {
      t,
      power: Math.round(plan + 6 * Math.sin(t * 1.7) + 3 * Math.sin(t * 0.45)),
      hr: Math.round(100 + 0.18 * t + 2 * Math.sin(t / 7)),
      cadence: Math.round(89 + 2 * Math.sin(t * 1.3) + Math.sin(t * 0.31)),
    };
  });

/** The devices of a loaded ride: the trainer's link before the start, and whether the strap linked at the load. */
interface Devices {
  trainer?: "ready" | "connecting" | "lost";
  /** The strap was not warm at the load: the ride does not hold it, the keeper has not found it. */
  strapMissed?: boolean;
}

/** A loaded ride in `over`'s state, 3:20 into its 10-minute workout (rideBackend `BLOCKS`), or a free ride. */
const ride =
  (
    over: Partial<RideWorld>,
    live: LiveValues = RIDING,
    workoutS = 200,
    workoutId: string | null = "w1",
    /** The engine is wedged (#351): the frame says so. */
    wedge = false,
    devices: Devices = {},
  ) =>
  () => {
    const rig = installRide(
      {
        blocks: BLOCKS,
        workoutId,
        live,
        trainer: devices.trainer ?? "ready",
        ...(devices.strapMissed
          ? {
              strapHanded: false,
              keeper: {
                trainer: { phase: "cold", sample: null },
                strap: { phase: "not_found", sample: null },
              },
            }
          : {}),
        ...over,
      },
      BEAT,
    );
    // The traces so far, as the engine appended them (ADR-0001 §4.3.3); the channel backfills them from index 0.
    for (const h of HISTORY(workoutS))
      for (const of of ["power", "hr", "cadence"] as const)
        rig.ui.truth.series[of].push({ t: h.t, w: h.t, v: h[of] });
    // This tick's values, which the engine appends as it publishes them (§4.3.3; a gap appends nothing).
    const tick = {
      power: live.powerW,
      hr: live.hrBpm,
      cadence: live.cadenceRpm,
    };
    if (workoutS > 0)
      for (const of of ["power", "hr", "cadence"] as const) {
        const v = tick[of];
        if (v !== null)
          rig.ui.truth.series[of].push({
            t: rig.world.sessionS,
            w: workoutS,
            v,
          });
      }
    if (wedge) rig.wedge();
    rig.publish(workoutS);
    navigate("ride");
  };
const active = { phase: "active", blockIdx: 1, sessionS: 200 } as const;

/**
 * An hour with blocks of every label width (DESIGN §18.3 "Strip labels"): a 30 s block too short even for its watts
 * (a leader), a 2-minute one that sheds its name at 1024, and long ones that show both.
 */
const SHORT_BLOCKS = [
  block(0, 600, 100, 180, "warmup"),
  block(600, 30, 400, 400, "work"),
  block(630, 270, 120, 120, "recovery"),
  block(900, 120, 300, 300, "work"),
  block(1020, 180, 120, 120, "recovery"),
  block(1200, 1800, 250, 250, "work"),
  block(3000, 600, 120, 100, "cooldown"),
];

/** Sweet spot 3x12 as the core compiles it: a warm-up ramp, three 12-minute sets at 90 % with 2 minutes easy, a cool-down. */
const SWEET_SPOT = compiled({
  blocks: [
    block(0, 240, 110, 175, "warmup"),
    ...[0, 1, 2].flatMap((i) => [
      block(240 + i * 840, 720, 225, 225, "work"),
      block(960 + i * 840, 120, 140, 140, "recovery"),
    ]),
    block(2760, 240, 140, 100, "cooldown"),
  ],
  totalS: 3000,
  totalRangeS: [3000, 3000],
  timeInZoneS: [180, 660, 0, 2160, 0, 0, 0],
});
/** A long name, and a long workout: 3.5 hours, 57 blocks, an open-ended cool-down (a duration range). */
const LONG_NAME =
  "Over-unders 6×(4×(2 min at 95 % FTP, 1 min at 110 % FTP)) with a long Zone 2 tail and an open-ended cool-down";
const LONG = (() => {
  const blocks = [block(0, 900, 100, 175, "warmup")];
  let t = 900;
  for (let set = 0; set < 6; set++) {
    for (let rep = 0; rep < 4; rep++) {
      blocks.push(block(t, 120, 238, 238, "work"), block(t + 120, 60, 275));
      t += 180;
    }
    blocks.push(block(t, 300, 140, 140, "recovery"));
    t += 300;
  }
  blocks.push(block(t, 4800, 175, 175, "work"));
  t += 4800;
  blocks.push({
    ...block(t, 600, 140, 100, "cooldown"),
    durRangeS: [300, 900],
  });
  return compiled({
    blocks,
    totalS: t + 600,
    totalRangeS: [t + 300, t + 900],
    tss: 251.6,
    if: 0.85,
    timeInZoneS: [600, 1500, 5400, 2880, 1440, 1440, 0],
    issues: [WARNING],
  });
})();
const workouts =
  (over: Partial<WorkoutsBackend> = {}) =>
  () => {
    installWorkouts({
      ui: base(),
      compiled: { "w-ss": SWEET_SPOT, "w-easy": compiled({ tss: 20 }) },
      ...over,
    });
    navigate("workouts");
  };

/** A 3 h ride: a warm-up, 18 sets of 6 minutes at 240 W with 3 easy, a cool-down; 57 blocks, two short of plan. */
const LONG_RIDE: RideRecord = (() => {
  const blocks: BlockSummary[] = [
    { index: 0, plannedS: 600, plannedW: 150, actualS: 600, actualW: 151 },
  ];
  for (let i = 0; i < 18; i++)
    blocks.push(
      {
        index: 1 + 3 * i,
        plannedS: 360,
        plannedW: 240,
        actualS: 360,
        actualW: 238 + (i % 5),
      },
      {
        index: 2 + 3 * i,
        plannedS: 120,
        plannedW: 140,
        actualS: 120,
        actualW: 141,
      },
      {
        index: 3 + 3 * i,
        plannedS: 60,
        plannedW: 120,
        actualS: i === 17 ? 40 : 60,
        actualW: i === 17 ? null : 118,
      },
    );
  blocks.push({
    index: 55,
    plannedS: 600,
    plannedW: 140,
    actualS: 600,
    actualW: 139,
  });
  blocks.push({
    index: 56,
    plannedS: 480,
    plannedW: 100,
    actualS: 140,
    actualW: 104,
  });
  return {
    id: "ride-long",
    startedAt: 1_761_000_000,
    endedAt: 1_761_010_800,
    outcome: "finished",
    workoutId: "w-long",
    workoutName: "Endurance 18×6 min at 240 W",
    summary: {
      durationS: 10_800,
      avgPowerW: 196.8,
      npW: 214.3,
      avgHrBpm: 138.6,
      kj: 2125.4,
      blocks,
    },
  };
})();

/** A 5-minute spin whose figures match the traces `rodeIt` draws for it. */
const SHORT_RIDE: RideRecord = {
  id: "ride-short",
  startedAt: 1_761_100_000,
  endedAt: 1_761_100_300,
  outcome: "finished",
  workoutId: null,
  workoutName: "Free ride",
  summary: {
    durationS: 300,
    avgPowerW: 140.2,
    npW: 141.0,
    avgHrBpm: 110.4,
    kj: 42.1,
    blocks: [],
  },
};

/**
 * The snapshot channel holding the ended ride `rideId` with `seconds` of its series, from fixed formulas (no
 * randomness): power at 4 Hz around a 6-minute pattern, heart rate and cadence at 2 Hz (ADR-0001 §4.3.3); the channel
 * backfills them, 256 points per series per frame. `ended` is how it ended.
 */
const rodeIt = (
  rideId: string,
  seconds: number,
  ended: RideView["ended"] = "finished",
) => {
  const at = (hz: number, v: (t: number) => number) =>
    Array.from({ length: seconds * hz }, (_, i) => {
      const t = i / hz;
      return { t, w: t, v: Math.round(v(t)) };
    });
  const work = (t: number) => (t % 540 < 360 && t > 600 ? 1 : 0);
  installUi({
    ...BEAT,
    ride: rideView({
      id: rideId,
      workoutId: null,
      status: makeStatus({ state: { kind: "finished" } }),
      ended,
    }),
    devices: devicesOf({ trainer: neo, strap: hrm }),
    series: {
      power: at(
        4,
        (t) =>
          140 + 100 * work(t) + 8 * Math.sin(t * 1.7) + 4 * Math.sin(t * 0.45),
      ),
      hr: at(
        2,
        (t) =>
          110 +
          35 * work(t) * Math.min(1, (t % 540) / 120) +
          3 * Math.sin(t / 7),
      ),
      cadence: at(
        2,
        (t) => 88 + 3 * Math.sin(t * 1.3) + 2 * Math.sin(t * 0.31),
      ),
    },
  });
};
const history =
  (sub?: string, over: Partial<HistoryBackend> = {}) =>
  () => {
    installHistory(over);
    navigate("history", sub);
  };
/** Forty rides, one a day back from 2026-11-01; every fifth was aborted and has no summary. */
const MANY: RideRecord[] = Array.from({ length: 40 }, (_, i) => ({
  ...RIDES[i % 2 === 0 ? 0 : 1],
  id: `ride-${100 + i}`,
  startedAt: 1_762_000_000 - i * 86_400,
  endedAt: 1_762_003_000 - i * 86_400,
  ...(i % 5 === 4 ? { outcome: "aborted" as const, summary: null } : {}),
}));

/** 2026-11-02T00:00Z, the day of the fixtures' frozen clock (design.pw.ts `FROZEN`), in Unix seconds. */
const MIDNIGHT = 1_793_577_600;
const HOUR = 3600;
/** A week of rides up to the frozen clock: the newest yesterday evening, one aborted with no summary. */
const WEEK: RideRecord[] = [
  { ...RIDES[0], id: "w-1", startedAt: MIDNIGHT - 6.5 * HOUR },
  {
    ...RIDES[0],
    id: "w-2",
    startedAt: MIDNIGHT - 2 * 24 * HOUR + 7 * HOUR,
    workoutName: "Endurance 90",
    summary: RIDES[0].summary && { ...RIDES[0].summary, durationS: 5400 },
  },
  { ...RIDES[2], id: "w-3", startedAt: MIDNIGHT - 4 * 24 * HOUR + 18 * HOUR },
  {
    ...RIDES[0],
    id: "w-4",
    startedAt: MIDNIGHT - 6 * 24 * HOUR + 6 * HOUR,
    workoutName: "Easy spin",
    summary: RIDES[0].summary && { ...RIDES[0].summary, durationS: 1800 },
  },
  ...RIDES,
];
/** Today's devices: what is paired, and the loaded ride's link of each device it holds. */
interface TodayDevices {
  paired?: { trainer?: PairedDevice; strap?: PairedDevice };
  links?: { trainer?: LinkStatus; strap?: LinkStatus };
  /** The snapshot channel sends nothing yet (its first frame is on its way). */
  quiet?: boolean;
  /** The channel cannot open: the keeper cannot read the pairings (`ui_open` answers `storage`). */
  keeperStorage?: boolean;
  /** The keeper's phase of each paired device with no ride (`cold` unless given). */
  phases?: { trainer?: LinkPhase; strap?: LinkPhase };
}

/**
 * Today on the core's reads: the rides, and the devices on the snapshot channel; `load` starts a ride load from
 * Workouts first (the workouts backend answers it and shows it on the channel).
 */
const today =
  (
    over: Parameters<typeof installToday>[0] = {},
    load?: Partial<WorkoutsBackend>,
    dev: TodayDevices = {},
  ) =>
  () => {
    navigate("today");
    const ui = installUi({
      ...BEAT,
      devices: devicesOf(
        dev.paired ?? { trainer: neo, strap: hrm },
        {},
        dev.phases,
      ),
      keeperStorage: dev.keeperStorage ?? false,
    });
    if (dev.quiet) ui.stall();
    if (load) installWorkouts({ ...load, ui });
    installToday({ rides: WEEK, others: internals().invoke, ...over });
    if (load)
      void startRideLoad("w-long", LONG_NAME).then(() => {
        if (dev.links) {
          ui.truth.devices = devicesOf(
            dev.paired ?? { trainer: neo, strap: hrm },
            dev.links,
          );
          ui.changed();
        }
      });
  };
const longLibrary = (
  over: Partial<WorkoutsBackend> = {},
): Partial<WorkoutsBackend> => ({
  summaries: [
    ...SUMMARIES,
    {
      id: "w-long",
      name: LONG_NAME,
      totalS: 12420,
      tss: 251.6,
      ftp: 250,
      builtin: false,
    },
  ],
  compiled: { "w-long": LONG },
  ...over,
});

const SETUPS: Record<ScenarioName, () => void> = {
  today: today(),
  "today-empty": today({ rides: [] }, undefined, { paired: {} }),
  // The rides are still on the wire, and the channel's first frame too.
  "today-reading": today(
    { hold: { rides_list: new Promise<void>(() => {}) } },
    undefined,
    { quiet: true },
  ),
  "today-error": today(
    { fail: { rides_list: { code: "storage", message: "disk I/O error" } } },
    undefined,
    { keeperStorage: true },
  ),
  // The load never answers: Start reads "Loading ride…".
  "today-ride-loading": today(
    {},
    longLibrary({ gate: new Promise<void>(() => {}) }),
  ),
  "today-ride-loaded": today({}, longLibrary(), {
    links: { trainer: "ready", strap: "connecting" },
  }),
  // No trainer is paired in the workouts backend's core: `ride_load` is refused with `device_unavailable`.
  "today-ride-refused": today({}, longLibrary({ trainerPaired: false })),
  "today-link-lost": today({}, longLibrary(), {
    links: { trainer: "lost", strap: "ready" },
  }),
  // The link chips (ADR-0002 §5.1.4, #681): the keeper warms both devices up on Today, a warm view.
  "today-links-warming": today({}, undefined, {
    phases: { trainer: "searching", strap: "connecting" },
  }),
  // The trainer is not found (another app may hold it): ⚠ with the hedged hint, and Retry; the strap is ready.
  "today-links-not-found": today({}, undefined, {
    phases: { trainer: "not_found", strap: "warm" },
  }),
  // The library as the core lists it (the built-in one is never empty today; the store-backed one, #110, can be).
  "workouts-empty": workouts({ summaries: [] }),
  // The list cannot be read (Tauri's own rejection, or the store-backed library to come).
  "workouts-list-error": workouts({
    fail: { workouts_list: { code: "storage", message: "database is locked" } },
  }),
  workouts: workouts(),
  "workouts-selected": workouts(),
  // The link chips under Ride this (#681): the trainer in its handshake, the strap not found with its hint, and Retry.
  "workouts-links": () => {
    workouts({
      ui: installUi({
        ...BEAT,
        devices: devicesOf(
          { trainer: neo, strap: hrm },
          {},
          { trainer: "handshake", strap: "not_found" },
        ),
      }),
    })();
  },
  // An error-severity issue: the core refuses to load it (core.rs `Core::resolve_workout`), so Ride this is disabled.
  "workouts-blocked": workouts({
    compiled: { "w-ss": { ...SWEET_SPOT, issues: [ERROR_ISSUE, WARNING] } },
  }),
  // Ride this was pressed and the load never answers: the library is locked, Ride this pending.
  "workouts-loading": workouts({ gate: new Promise<void>(() => {}) }),
  // The load was refused: no trainer is paired.
  "workouts-load-error": workouts({ trainerPaired: false }),
  "workouts-long": workouts({
    summaries: [
      ...SUMMARIES,
      {
        id: "w-long",
        name: LONG_NAME,
        totalS: 12420,
        tss: 251.6,
        ftp: 250,
        builtin: false,
      },
    ],
    compiled: { "w-ss": SWEET_SPOT, "w-long": LONG },
  }),
  // The editor on the core's `sweet-spot` draft (editorFixture.ts): a real tree, text and compiled view.
  "workouts-editor": workouts({ trees: { "w-ss": "sweet-spot" } }),
  "workouts-editor-invalid": workouts({ trees: { "w-ss": "sweet-spot" } }),
  "workouts-editor-step": workouts({ trees: { "w-ss": "sweet-spot" } }),
  "workouts-editor-repeat": workouts({ trees: { "w-ss": "sweet-spot" } }),
  // The refused edit is the core's `zero-duration` tree (editor.json): its printed text does not read back.
  "workouts-editor-refused": workouts({ trees: { "w-ss": "sweet-spot" } }),
  "workouts-mine": workouts(),
  "workouts-delete-confirm": workouts(),
  // The new workout's empty tree is the core's `empty` draft (editorFixture.ts): compiled, with its error.
  "workouts-editor-new": workouts(),
  "devices-empty": devices({}),
  // The scan never answers: the button stays "Scanning…".
  "devices-scanning": devices({ gate: new Promise<void>(() => {}) }),
  // The scan answered and nothing is paired: each device offers its role.
  "devices-found": devices({}),
  // The pair never answers: its button stays "Pairing…" and every other is disabled.
  "devices-pairing": devices({ pairGate: new Promise<void>(() => {}) }),
  "devices-paired": devices({ paired: { trainer: neo, heart_rate: hrm } }),
  // Forget asks first: the trainer row holds the question.
  "devices-forget-confirm": devices({
    paired: { trainer: neo, heart_rate: hrm },
  }),
  "devices-denied": devices({
    adapter: { code: "permission_denied", message: "Bluetooth denied" },
  }),
  // The Ready sheet (DESIGN §5.4, #526): the loaded, unstarted ride in each of its states.
  ready: ride({ phase: "loaded" }, NO_LIVE, 0),
  // This window's `ride_load` of w1 is under way and never answers: the channel's frame says `loading` and has no ride
  // yet (ui.rs `Ui::load_started`), and the screen draws the Ready sheet's loading state from it, with the plan of the
  // workout asked for (`useRideLoad`).
  "ready-loading": () => {
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
    // The ride commands, `workout_get` among them, are the ride backend's; `ride_load` falls through to the library's.
    const rig = installRide({ phase: "none", blocks: BLOCKS }, BEAT);
    library.ui = rig.ui;
    void startRideLoad("w1", "Steady 10 min");
    navigate("ride");
  },
  // The same load over cold devices (#681): each row shows its device's keeper phase, the trainer searching and the
  // strap not found.
  "ready-loading-progress": () => {
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
    const rig = installRide({ phase: "none", blocks: BLOCKS }, BEAT);
    library.ui = rig.ui;
    rig.setKeeper({
      trainer: { phase: "searching", sample: null },
      strap: { phase: "not_found", sample: null },
    });
    void startRideLoad("w1", "Steady 10 min");
    navigate("ride");
  },
  // The trainer link is coming back up; the core refuses Start until it is ready (rideBackend `trainer`).
  "ready-connecting": ride({ phase: "loaded" }, NO_LIVE, 0, "w1", false, {
    trainer: "connecting",
  }),
  "ready-trainer-missing": ride({ phase: "loaded" }, NO_LIVE, 0, "w1", false, {
    trainer: "lost",
  }),
  // The strap did not link at the load: the ride runs without heart rate.
  "ready-strap-missing": ride({ phase: "loaded" }, NO_LIVE, 0, "w1", false, {
    strapMissed: true,
  }),
  // Start was pressed and the core refused it: the record cannot be opened.
  "ready-error": ride({ phase: "loaded", recordFails: true }, NO_LIVE, 0),
  // Back 6 s after a 4 s loss, ramping to the plan from 120 W, two of its five steps taken
  // (erg/policy.rs `ErgPolicy::desired`, the soft start).
  "ride-soft-start": ride({
    ...active,
    sessionS: 206,
    softStartW: 250,
    gapS: 4,
    reconnectedAtS: 200,
    softOwed: true,
    rampOrigin: 120,
    rampStep: 2,
  }),
  "ride-work-step": ride(active),
  // The HR zone (DESIGN §18.3 "Heart-rate zone", #586): 142 bpm is 101 % of an LTHR of 140, Friel's Z5a, the widest
  // label ("Z5a · LTHR"); and with only an HR max of 190 it is 75 %, Z3 of the five zones.
  "ride-hr-zone": () => {
    setAthleteHr({ lthrBpm: 140, hrMaxBpm: 190 });
    ride(active)();
  },
  "ride-hr-zone-max": () => {
    setAthleteHr({ hrMaxBpm: 190 });
    ride(active)();
  },
  "ride-short-block": ride({
    phase: "active",
    blockIdx: 0,
    sessionS: 200,
    blocks: SHORT_BLOCKS,
    workoutName: "Hour with a sprint",
  }),
  "ride-paused": ride(
    { ...active, userPause: true },
    { ...RIDING, powerW: 0, cadenceRpm: 0 },
  ),
  "ride-hr-lost": ride({ ...active, hrLost: true }, { ...RIDING, hrBpm: null }),
  "ride-trainer-lost": ride(
    {
      ...active,
      sessionS: 212,
      trainerLost: true,
      lostSinceS: 200,
      attempt: 12,
    },
    { ...RIDING, powerW: null, cadenceRpm: null },
  ),
  // The longest trainer-loss copy (banners.tsx): no samples on a ready link with the strap lost, 59:59 into the loss,
  // so the bold line is its widest and End… and what to try show.
  "ride-trainer-lost-long": ride(
    {
      ...active,
      sessionS: 3799,
      trainerSilent: true,
      hrLost: true,
      lostSinceS: 200,
    },
    { powerW: null, cadenceRpm: null, hrBpm: null },
  ),
  // Two banners at once: the engine stops answering while the trainer is lost (#351, §6.5).
  "ride-engine-silent": ride(
    {
      ...active,
      sessionS: 212,
      trainerLost: true,
      lostSinceS: 200,
      attempt: 3,
    },
    { ...RIDING, powerW: null, cadenceRpm: null },
    200,
    "w1",
    true,
  ),
  "ride-paused-hr-lost": ride(
    { ...active, userPause: true, hrLost: true },
    { powerW: 0, cadenceRpm: 0, hrBpm: null },
  ),
  "ride-erg-released": ride(
    { ...active, ergReleased: true },
    { ...RIDING, powerW: 40, cadenceRpm: 31 },
  ),
  "ride-free": ride({ ...active, blocks: [] }, RIDING, 200, null),
  review: history("ride-1"),
  "review-short": () => {
    rodeIt("ride-short", 300);
    history("ride-short", { rides: [SHORT_RIDE, ...RIDES] })();
  },
  "review-long": () => {
    rodeIt("ride-long", 10_800);
    history("ride-long", { rides: [LONG_RIDE, ...RIDES] })();
  },
  // The core ended ride-2 by force (`ride.ended: forced`); the channel keeps it until the next load. Dev's fixture marked
  // the forced end and had no traces for the ride, so the frame carries the ride and no points.
  "review-aborted": () => {
    rodeIt("ride-2", 0, "forced");
    history("ride-2")();
  },
  "review-error": history("ride-1", {
    fail: { ride_summary: { code: "storage", message: "database is locked" } },
  }),
  "history-empty": history(undefined, { rides: [] }),
  "history-error": history(undefined, {
    fail: { rides_list: { code: "storage", message: "disk I/O error" } },
  }),
  history: history(),
  "history-long": history(undefined, { rides: MANY }),
  // Opened from Review: "All rides" leads back to the list with this ride marked.
  "history-selected": history("ride-3"),
  settings: () => navigate("settings"),
  // Neither HR reference known: the one-line hint (§18.3 "Heart-rate zone").
  "settings-athlete": () => navigate("settings", "athlete"),
  "settings-ride-loaded": () => {
    navigate("settings");
    installWorkouts({ ...longLibrary(), ui: base() });
    void startRideLoad("w-long", LONG_NAME);
  },
  // The destination (Today) is never reached; the start screen calls nothing.
  "start-slow": () => navigate("today"),
  "start-failed": () => navigate("today"),
};

// The start screen's fixtures boot slowly or fail; every other one is a core that is ready at once (core.ts).
const BOOTS: Partial<Record<ScenarioName, WaitForCore>> = {
  "start-slow": () => new Promise<void>(() => {}),
  "start-failed": () =>
    Promise.reject(new Error("its saved state could not be opened")),
};

/** The fixtures' build string (version.rs `BUILD_LABEL`'s shape); its spin-up counts as played (a later launch). */
const BUILD = "0.2.0-rc.1 · 7516045";
const readBuild = () => Promise.resolve(BUILD);
markSeen(BUILD);

const name = new URLSearchParams(location.search).get("scenario");
const scenario = SCENARIOS.find((s) => s.name === name);
if (!scenario) throw new Error(`unknown scenario ${String(name)}`);
// Screens that call nothing still get a backend: an unknown command is Tauri's plain-string rejection.
mockIPC((cmd) => Promise.reject(`command ${cmd} not found`));
baseUi = installUi(BEAT);
SETUPS[scenario.name]();

// Count the answers in flight, on top of whatever the scenario installed.
const counted = internals().invoke;
internals().invoke = (cmd, args) => {
  pending++;
  return counted(cmd, args).finally(() => pending--);
};
const frame = () => new Promise((r) => requestAnimationFrame(r));
(window as unknown as { __fixture: unknown }).__fixture = {
  /**
   * Resolves once every bundled font face (theme.css) is loaded, then two frames in a row saw no answer in flight
   * beyond the scenario's held ones. The faces are `font-display: swap` and fetched only when text first needs
   * them, so without the load a slow fetch left the audit measuring the fallback font (#504). A face that fails
   * to load rejects, and the test with it.
   */
  async settled() {
    await Promise.all([...document.fonts].map((f) => f.load()));
    const held = "held" in scenario ? scenario.held : 0;
    for (let quiet = 0; quiet < 2;) {
      await frame();
      quiet = pending <= held ? quiet + 1 : 0;
    }
  },
};

// Determinism for the visual baselines (#459): no motion at all (reduced motion already stops the one fade, this
// stops any later one) and no caret, so a screenshot never catches a frame mid-change.
const still = document.createElement("style");
still.textContent = `*, *::before, *::after {
  animation: none !important;
  transition: none !important;
  caret-color: transparent !important;
}`;
document.head.append(still);

const root = document.getElementById("root");
if (!root) throw new Error("Missing app root");
createRoot(root).render(
  <StrictMode>
    <AppRoot readBuild={readBuild} waitForCore={BOOTS[scenario.name]} />
  </StrictMode>,
);
