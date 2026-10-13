// The medical stop is a GLOBAL invariant (DESIGN §5.8), not a Today feature: every ride start in the
// app goes through requestStart(), the single guard. While a stop is active no planned or coached
// ride starts from any path; the only exception is the deliberate, logged free ride. Production
// enforces the same rule in the core, so MCP/CLI starts can't bypass it (#81).
import type { Workout } from "@crank/shared";
import { sceneFixture, type HealthAnswer, type HealthId } from "./mock/ext";
import { loadRide, navigate } from "./session";
import { createStore } from "./store";

export type StartPath = "today" | "workouts" | "resume" | "history" | "ready";
export const START_PATH_NAME: Record<StartPath, string> = { today: "Today", workouts: "Workouts", resume: "the Resume sheet", history: "History", ready: "the Ready check" };

/** stop: a red flag is on record; checkedToday: today's red-flag re-check came back all "no" */
export const safety = createStore<{ stop: boolean; checkedToday: boolean; symptomInRide: boolean }>({ stop: false, checkedToday: false, symptomInRide: false });
/** the start the guard is holding while it asks or blocks */
export const pendingStart = createStore<{ workout: Workout; from: StartPath } | null>(null);

export const stopActive = () => safety.get().stop || !!sceneFixture.get().medicalStop;
export const isFreeRide = (w: Workout) => w.id === "free";

/** The one guard every start path calls. */
export function requestStart(workout: Workout, from: StartPath) {
  if (isFreeRide(workout)) {
    // reached only through the ticked free-ride confirmation; logged with its reason
    loadRide(workout);
    navigate("ready");
    return;
  }
  if (stopActive()) {
    pendingStart.set({ workout, from });
    navigate("gate", "blocked");
    return;
  }
  if (!safety.get().checkedToday) {
    pendingStart.set({ workout, from });
    navigate("gate", "check");
    return;
  }
  loadRide(workout);
  navigate("ready");
}

/** Record today's red-flag re-check. Any "yes" sets the medical stop. */
export function recordRedFlagCheck(answers: Partial<Record<HealthId, HealthAnswer>>) {
  const yes = Object.values(answers).some((a) => a === "yes");
  safety.set((s) => (yes ? { ...s, stop: true } : { ...s, checkedToday: true }));
  return !yes;
}

/** In-ride symptom (§5.2): stop the workout safely and set the medical stop. */
export function reportSymptomInRide() {
  safety.set((s) => ({ ...s, stop: true, symptomInRide: true }));
}
