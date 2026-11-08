// The design fixture entry (#457): mounts the real `AppRoot` on the repo's mockIPC backends in the state that
// `?scenario=<name>` names (scenarios.ts). Dev and test only: it has its own Vite root (vite.config.ts here), so the
// production build (app/index.html) never sees it. Each screen installs only the backend it calls, so every refusal
// is the backend's own (their headers cite the core); a command no backend answers is Tauri's plain-string rejection.
// `window.__fixture.settled()` resolves once no mock answer is in flight (bar the scenario's `held` ones).
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import { AppRoot } from "../../src/App";
import "../../src/theme.css";
import "../../src/shell.css";
import type { LinkStatus, MetricId, PairedDevice } from "../../src/bindings";
import { navigate } from "../../src/router";
import {
  BLOCKS,
  block,
  installRide,
  type RideWorld,
} from "../../src/ride/rideBackend";
import { beginRide } from "../../src/ride/session";
import {
  installDevices,
  strap,
  trainer,
  type DevicesState,
} from "../../src/views/devicesBackend";
import { markForcedEnd } from "../../src/ride/forcedEnds";
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
import { startRideLoad } from "../../src/views/rideLoad";
import { SCENARIOS, type ScenarioName } from "./scenarios";

/** Mock answers (and fixture frames) in flight; see `settled()`. */
let pending = 0;

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

const devices = (over: Partial<DevicesState>) => () => {
  installDevices({ nearby: [trainer, strap], ...over });
  navigate("settings", "devices");
};

/** Live values per metric, set as the core's sample when the ride view subscribes. */
const RIDING: Partial<Record<MetricId, number | null>> = {
  power: 248,
  cadence: 89,
  hr: 142,
};

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

/** The devices of a loaded ride: each link as the core reports it (`null`: paired, no link), and the load's answer. */
interface Devices {
  trainer?: LinkStatus | null;
  hr?: LinkStatus | null;
  /** `RideLoaded.hrLinked`; defaults to a ready strap link. */
  hrLinked?: boolean;
  /** A stream command whose answer never comes (the scenario's `held`): `stream_snapshot` keeps the status unknown. */
  hold?: string;
}

/** A loaded ride in `over`'s state, 3:20 into its 10-minute workout (rideBackend `BLOCKS`), or a free ride. */
const ride =
  (
    over: Partial<RideWorld>,
    values = RIDING,
    workoutS = 200,
    workoutId: string | null = "w1",
    /** The engine wedges once the view has subscribed (#351): its next frame's ack is refused. */
    wedge = false,
    devices: Devices = {},
  ) =>
  () => {
    const rig = installRide({ blocks: BLOCKS, ...over });
    const { trainer: t = "ready", hr = "ready" } = devices;
    // a loaded or started ride holds both devices, which the keeper reports `riding`; after it they are `cold`
    // (core.rs `Core::device_links`)
    const held =
      (over.phase ?? "loaded") === "loaded" || over.phase === "active";
    const phase = held ? ("riding" as const) : ("cold" as const);
    rig.base.truth.infos = [
      { role: "trainer", device: neo, link: t, phase },
      { role: "heart_rate", device: hrm, link: hr, phase },
    ];
    rig.publish(workoutS);
    // Before Start the core reports the loaded ride's links in the snapshot as `device_links` does.
    if (over.phase === "loaded")
      rig.base.truth.links = [
        ...(t === null ? [] : [{ device: "trainer" as const, state: t }]),
        ...(hr === null ? [] : [{ device: "hr" as const, state: hr }]),
      ];
    if (devices.hold) rig.base.hold(devices.hold);
    // The ride view's subscriptions get the scenario's values as the core's latest sample, read by the snapshot.
    const inner = internals().invoke;
    let subscribed = 0;
    mockIPC(async (cmd, payload) => {
      const answer = await inner(cmd, payload);
      if (cmd === "stream_subscribe") {
        const metric = (payload as { metric: MetricId }).metric;
        const sample = {
          sub: answer as number,
          t: rig.world.sessionS,
          v: values[metric] ?? null,
        };
        rig.base.setSample(sample);
        // The view takes values from frames (the snapshot is read on a re-query only): send one once the client
        // holds the answer, i.e. after its continuation, counted as in flight so `settled()` waits for it.
        pending++;
        setTimeout(() => {
          rig.base.push({ samples: [sample] });
          if (wedge && ++subscribed === 3) {
            rig.wedge();
            rig.base.push({ status: rig.base.truth.status });
          }
          pending--;
        });
      }
      return answer;
    });
    const session = beginRide({
      rideId: rig.world.rideId,
      workoutId,
      hrLinked: devices.hrLinked ?? hr === "ready",
    });
    for (const h of HISTORY(workoutS))
      for (const of of ["power", "hr", "cadence"] as const)
        session.record(h.t, h.t, h[of], of);
    navigate("ride");
  };
const active = { phase: "active", blockIdx: 1, sessionS: 200 } as const;

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
 * The window's session of `rideId` holding `seconds` of its traces, from fixed formulas (no randomness): power at
 * 4 Hz around a 6-minute pattern, heart rate and cadence at 2 Hz. Set at once, as `record` would leave them
 * (appending each sample is #557's O(n²), not what a fixture shows).
 */
const rodeIt = (rideId: string, seconds: number) => {
  const session = beginRide({ rideId, workoutId: null, hrLinked: true });
  const at = (hz: number, v: (t: number) => number) =>
    Array.from({ length: seconds * hz }, (_, i) => {
      const t = i / hz;
      return { t, workoutS: t, v: Math.round(v(t)) };
    });
  const work = (t: number) => (t % 540 < 360 && t > 600 ? 1 : 0);
  session.series = {
    power: at(
      4,
      (t) =>
        140 + 100 * work(t) + 8 * Math.sin(t * 1.7) + 4 * Math.sin(t * 0.45),
    ),
    hr: at(
      2,
      (t) =>
        110 + 35 * work(t) * Math.min(1, (t % 540) / 120) + 3 * Math.sin(t / 7),
    ),
    cadence: at(2, (t) => 88 + 3 * Math.sin(t * 1.3) + 2 * Math.sin(t * 0.31)),
  };
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
/** Today on the core's reads; `load` starts a ride load from Workouts first (the workouts backend answers it). */
const today =
  (
    over: Parameters<typeof installToday>[0] = {},
    load?: Partial<WorkoutsBackend>,
  ) =>
  () => {
    navigate("today");
    let others: Invoke | null = null;
    if (load) {
      installWorkouts(load);
      others = internals().invoke;
    }
    installToday({
      rides: WEEK,
      paired: { trainer: neo, heart_rate: hrm },
      others,
      ...over,
    });
    if (load) void startRideLoad("w-long", LONG_NAME);
  };
const longLibrary = (
  over: Partial<WorkoutsBackend> = {},
): Partial<WorkoutsBackend> => ({
  summaries: [
    ...SUMMARIES,
    { id: "w-long", name: LONG_NAME, totalS: 12420, tss: 251.6, ftp: 250 },
  ],
  compiled: { "w-long": LONG },
  ...over,
});

const SETUPS: Record<ScenarioName, () => void> = {
  today: today(),
  "today-empty": today({ rides: [], paired: {} }),
  "today-reading": today({
    hold: {
      rides_list: new Promise<void>(() => {}),
      device_links: new Promise<void>(() => {}),
    },
  }),
  "today-error": today({
    fail: {
      rides_list: { code: "storage", message: "disk I/O error" },
      device_links: { code: "storage", message: "database is locked" },
    },
  }),
  // The load never answers: Start reads "Loading ride…".
  "today-ride-loading": today(
    {},
    longLibrary({ gate: new Promise<void>(() => {}) }),
  ),
  "today-ride-loaded": today(
    { links: { trainer: "ready", heart_rate: "connecting" } },
    longLibrary(),
  ),
  // No trainer is paired in the workouts backend's core: `ride_load` is refused with `device_unavailable`.
  "today-ride-refused": today({}, longLibrary({ trainerPaired: false })),
  "today-link-lost": today(
    { links: { trainer: "lost", heart_rate: "ready" } },
    longLibrary(),
  ),
  // The library as the core lists it (the built-in one is never empty today; the store-backed one, #110, can be).
  "workouts-empty": workouts({ summaries: [] }),
  // The list cannot be read (Tauri's own rejection, or the store-backed library to come).
  "workouts-list-error": workouts({
    fail: { workouts_list: { code: "storage", message: "database is locked" } },
  }),
  workouts: workouts(),
  "workouts-selected": workouts(),
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
      { id: "w-long", name: LONG_NAME, totalS: 12420, tss: 251.6, ftp: 250 },
    ],
    compiled: { "w-ss": SWEET_SPOT, "w-long": LONG },
  }),
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
  ready: ride({ phase: "loaded" }, {}, 0),
  // The stream has not answered: no status yet (its snapshot is held), so nothing is the core's.
  "ready-loading": ride({ phase: "loaded" }, {}, 0, "w1", false, {
    hold: "stream_snapshot",
  }),
  // The trainer link is coming back up; the core refuses Start until it is ready (rideBackend `trainer`).
  "ready-connecting": ride(
    { phase: "loaded", trainer: "connecting" },
    {},
    0,
    "w1",
    false,
    { trainer: "connecting" },
  ),
  "ready-trainer-missing": ride(
    { phase: "loaded", trainer: "connecting" },
    {},
    0,
    "w1",
    false,
    { trainer: "lost" },
  ),
  // The strap did not link at the load: the ride runs without heart rate.
  "ready-strap-missing": ride({ phase: "loaded" }, {}, 0, "w1", false, {
    hr: null,
    hrLinked: false,
  }),
  // Start was pressed and the core refused it: the record cannot be opened.
  "ready-error": ride({ phase: "loaded", recordFails: true }, {}, 0),
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
  "ride-paused": ride(
    { ...active, userPause: true },
    { ...RIDING, power: 0, cadence: 0 },
  ),
  "ride-hr-lost": ride({ ...active, hrLost: true }, { ...RIDING, hr: null }),
  "ride-trainer-lost": ride(
    {
      ...active,
      sessionS: 212,
      trainerLost: true,
      lostSinceS: 200,
      attempt: 12,
    },
    { ...RIDING, power: null, cadence: null },
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
    { ...RIDING, power: null, cadence: null, hr: null },
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
    { ...RIDING, power: null, cadence: null },
    200,
    "w1",
    true,
  ),
  "ride-paused-hr-lost": ride(
    { ...active, userPause: true, hrLost: true },
    { ...RIDING, power: 0, cadence: 0, hr: null },
  ),
  "ride-erg-released": ride(
    { ...active, ergReleased: true },
    { ...RIDING, power: 40, cadence: 31 },
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
  "review-aborted": () => {
    markForcedEnd("ride-2");
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
  "settings-ride-loaded": () => {
    navigate("settings");
    installWorkouts(longLibrary());
    void startRideLoad("w-long", LONG_NAME);
  },
};

const name = new URLSearchParams(location.search).get("scenario");
const scenario = SCENARIOS.find((s) => s.name === name);
if (!scenario) throw new Error(`unknown scenario ${String(name)}`);
// Screens that call nothing still get a backend: an unknown command is Tauri's plain-string rejection.
mockIPC((cmd) => Promise.reject(`command ${cmd} not found`));
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
    <AppRoot />
  </StrictMode>,
);
