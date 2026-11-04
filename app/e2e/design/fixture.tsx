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
import type { MetricId, PairedDevice } from "../../src/bindings";
import { navigate } from "../../src/router";
import {
  BLOCKS,
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
import { install as installHistory } from "../../src/views/historyBackend";
import {
  compiled,
  install as installWorkouts,
} from "../../src/views/workoutsBackend";
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

/** A loaded ride in `over`'s state, 3:20 into its 10-minute workout (rideBackend `BLOCKS`). */
const ride =
  (over: Partial<RideWorld>, values = RIDING, workoutS = 200) =>
  () => {
    const rig = installRide({ blocks: BLOCKS, ...over });
    rig.base.truth.infos = [
      { role: "trainer", device: neo, link: "ready" },
      { role: "heart_rate", device: hrm, link: "ready" },
    ];
    rig.publish(workoutS);
    // The ride view's subscriptions get the scenario's values as the core's latest sample, read by the snapshot.
    const inner = internals().invoke;
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
          pending--;
        });
      }
      return answer;
    });
    beginRide({ rideId: rig.world.rideId, workoutId: "w1", hrLinked: true });
    navigate("ride");
  };
const active = { phase: "active", blockIdx: 1, sessionS: 200 } as const;

const SETUPS: Record<ScenarioName, () => void> = {
  today: () => navigate("today"),
  workouts: () => {
    installWorkouts({ compiled: { "w-ss": compiled({ blocks: BLOCKS }) } });
    navigate("workouts");
  },
  "workouts-selected": () => SETUPS.workouts(),
  "devices-empty": devices({}),
  // The scan never answers: the button stays "Scanning…".
  "devices-scanning": devices({ gate: new Promise<void>(() => {}) }),
  "devices-paired": devices({ paired: { trainer: neo, heart_rate: hrm } }),
  "devices-denied": devices({
    adapter: { code: "permission_denied", message: "Bluetooth denied" },
  }),
  ready: ride({ phase: "loaded" }, {}, 0),
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
      attempt: 2,
    },
    { ...RIDING, power: null, cadence: null },
  ),
  review: () => {
    installHistory();
    navigate("history", "ride-1");
  },
  history: () => {
    installHistory();
    navigate("history");
  },
  settings: () => navigate("settings"),
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

const root = document.getElementById("root");
if (!root) throw new Error("Missing app root");
createRoot(root).render(
  <StrictMode>
    <AppRoot />
  </StrictMode>,
);
