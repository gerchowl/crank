// Deterministic scenes for the screenshot harness (DESIGN §12). `?scene=<id>` builds the seed-42
// mock on a virtual clock, advances it to the scene's moment and freezes it there.
import { demoWorkout, parseDsl, type MockBackend, type RideStatus, type Workout } from "@crank/shared";
import { editor } from "./editor/model";
import { pathNodes } from "./format";
import { sceneFixture, type SceneFixture } from "./mock/ext";
import { backend, loadRide, navigate, session, type View } from "./session";

const SPRINT_DSL = `"Warm-up" 6m ramp 45-70% 90rpm
"Sprints" 3x(15s 180% 110-120rpm, "Easy" 4m45s 50%)
"Cool-down" 5m ramp 55-40%`;

function sprintWorkout(): Workout {
  const r = parseDsl(SPRINT_DSL, backend.athlete.ftp);
  if ("error" in r) throw new Error(r.error.message);
  return { id: "sprints", name: "Sprint primer 3×15″", ftp: backend.athlete.ftp, nodes: r.nodes };
}

const status = (): RideStatus => {
  const s = session.get()?.status.get();
  if (!s) throw new Error("scene: no ride session");
  return s;
};

/** Ride forward in 0.25 s steps until `done` holds (ride seconds cap). */
function rideUntil(b: MockBackend, done: (s: RideStatus) => boolean, capS = 3600) {
  for (let i = 0; i < capS * 4; i++) {
    if (done(status())) return;
    if (status().state !== "riding") throw new Error(`scene: ride ${status().state} before condition`);
    b.advance(0.25);
  }
  throw new Error("scene: condition not reached");
}
const rideFor = (b: MockBackend, s: number) => {
  for (let i = 0; i < s * 4; i++) b.advance(0.25);
};

const blockOf = (s: RideStatus) => session.get()?.current.get()?.compiled.blocks[s.blockIndex];
const labelled = (s: RideStatus, label: string) => {
  const c = session.get()?.current.get();
  const b = blockOf(s);
  return !!c && !!b && pathNodes(c.workout.nodes, b.path).some((n) => n.label === label);
};
const nudgeIs = (st: string) => () => session.get()?.nudges.get().some((n) => n.status === st) ?? false;

function startRide(w: Workout = demoWorkout(backend.athlete.ftp)) {
  loadRide(w);
  backend.ride.start();
}

interface Scene { view: View; sub?: string; setup?: () => void }
const ride = (setup: () => void): Scene => ({ view: "ride", setup: () => (startRide(), setup()) });
/** A state the mock cannot produce: ride to a moment, then overlay what the core would send (labelled fixture). */
const fixture = (setup: () => void, fx: SceneFixture): Scene => ({ view: "ride", setup: () => (startRide(), setup(), sceneFixture.set(fx)) });
const atThreshold = () => (rideUntil(backend, (s) => labelled(s, "Threshold") && s.phase === "work"), rideFor(backend, 40));

export const scenes: Record<string, Scene> = {
  "onboarding-about": { view: "onboarding", sub: "about" },
  "onboarding-background": { view: "onboarding", sub: "background" },
  "onboarding-numbers": { view: "onboarding", sub: "numbers" },
  "onboarding-health": { view: "onboarding", sub: "health" },
  "onboarding-devices": { view: "onboarding", sub: "devices" },
  "onboarding-baseline": { view: "onboarding", sub: "baseline" },
  "onboarding-result": { view: "onboarding", sub: "result" },
  "onboarding-resume": { view: "onboarding", sub: "resume" },
  today: { view: "today" },
  "today-health-cap": { view: "today", setup: () => sceneFixture.set({ healthCap: true }) },
  ready: { view: "ready", setup: () => loadRide(demoWorkout(backend.athlete.ftp)) },
  "ready-no-trainer": { view: "ready", setup: () => (loadRide(demoWorkout(backend.athlete.ftp)), sceneFixture.set({ status: { trainer: "lost" } })) },
  "ready-no-hr": { view: "ready", setup: () => (loadRide(demoWorkout(backend.athlete.ftp)), sceneFixture.set({ status: { hrSensor: "lost" } })) },
  "ride-warmup-ramp": ride(() => rideUntil(backend, (s) => s.t >= 300)),
  "ride-vo2-on": ride(() => (rideUntil(backend, (s) => s.t > 1000 && s.phase === "work" && blockOf(s)?.durS === 30), rideFor(backend, 9))),
  "ride-threshold": ride(() => (rideUntil(backend, (s) => labelled(s, "Threshold") && s.phase === "work"), rideFor(backend, 95))),
  "ride-sprint": {
    view: "ride",
    setup: () => (startRide(sprintWorkout()), rideUntil(backend, (s) => labelled(s, "Sprints") && s.phase === "work"), rideFor(backend, 6)),
  },
  "ride-hr-recovery": ride(() => (rideUntil(backend, (s) => labelled(s, "Recover")), rideFor(backend, 55))),
  "ride-coach-ask": ride(() => (rideUntil(backend, (s) => s.t > 2000 && nudgeIs("pending")()), rideFor(backend, 6))),
  "ride-coach-applied": ride(() => (rideUntil(backend, nudgeIs("auto_applied")), rideFor(backend, 4))),
  "ride-trainer-lost": ride(() => (rideUntil(backend, (s) => s.trainer !== "connected"), rideFor(backend, 3))),
  "ride-reconnected": ride(() => (rideUntil(backend, (s) => s.trainer !== "connected"), rideUntil(backend, (s) => s.trainer === "connected"), rideFor(backend, 2))),
  "ride-both-lost": fixture(atThreshold, { status: { trainer: "lost", hrSensor: "lost" }, facts: { trainerLost: true, hrLost: true, lostForS: 14, reconnectAttempt: 5 } }),
  "ride-erg-released": fixture(atThreshold, { facts: { ergReleased: true } }),
  "ride-hr-lost": ride(() => (rideUntil(backend, (s) => s.t > 1150 && s.hrSensor !== "connected"), rideFor(backend, 1))),
  "ride-hr-lost-recovery": ride(() => (rideUntil(backend, (s) => labelled(s, "Recover") && s.hrSensor !== "connected"), rideFor(backend, 1))),
  "ride-paused": fixture(() => (atThreshold(), backend.ride.pause()), { facts: { pausedForS: 42 } }),
  "ride-feel-prompt": fixture(() => (rideUntil(backend, (s) => labelled(s, "Threshold") && s.phase !== "work"), rideFor(backend, 4)), { feelPrompt: true }),
  "ride-screen-restarted": fixture(atThreshold, { screenCrashed: true }),
  resume: { view: "resume" },
  review: { view: "review", setup: () => (startRide(), rideUntil(backend, (s) => s.state === "finished", 4000)) },
  "review-call-open": { view: "review", setup: () => (startRide(), rideUntil(backend, (s) => s.state === "finished", 4000), sceneFixture.set({ openCall: 1 })) },
  history: { view: "history", setup: () => (startRide(), rideUntil(backend, (s) => s.state === "finished", 4000)) },
  workouts: { view: "workouts", setup: () => editor.select(2) },
  "workouts-ai-diff": { view: "workouts", setup: () => (editor.select(2), editor.requestRewrite("harder")) },
  "settings-devices": { view: "settings", sub: "devices" },
  "settings-coach": { view: "settings", sub: "coach" },
  "settings-controls": { view: "settings", sub: "controls" },
  "settings-ride-screen": { view: "settings", sub: "ride-screen" },
};

export function applyScene(id: string) {
  const sc = scenes[id];
  if (!sc) throw new Error(`unknown scene "${id}" (known: ${Object.keys(scenes).join(", ")})`);
  sc.setup?.();
  navigate(sc.view, sc.sub);
}
