// The app's single backend instance, the route, and the per-ride session scope.
// DESIGN §10 #3: everything that belongs to one ride (status, coach calls, compiled plan, undo
// windows, sensor gaps) lives in one RideSession, created on ride.load and disposed on the next.
// DESIGN §10 #6: the session routes to Review when the ride finishes.
import { useEffect, useState } from "react";
import { createMockBackend, type CoachNudge, type MetricId, type MockBackend, type RideStatus, type Sample, type Workout } from "@crank/shared";
import { CAP_PCT_CP, CapMonitor, DETECT_S, reconnectAttempt, sceneFixture, type CapExceeded, type CoreFacts } from "./mock/ext";
import { createStore, useStore } from "./store";

const params = new URLSearchParams(location.search);
export const sceneId = params.get("scene");
const speedParam = Number(params.get("speed"));

export const backend: MockBackend = createMockBackend({
  seed: 42,
  workout: null,
  clock: sceneId ? "virtual" : "realtime",
  speed: speedParam === 10 || speedParam === 60 ? speedParam : 1,
  storage: null, // the prototype persists nothing through the mock; see persist.ts
});

// ---------- routing ----------
export type View = "onboarding" | "resume" | "gate" | "today" | "ready" | "ride" | "review" | "history" | "workouts" | "settings";
export interface Route { view: View; sub?: string }
export const route = createStore<Route>({ view: "today" });
export const navigate = (view: View, sub?: string) => {
  route.set(sub === undefined ? { view } : { view, sub });
};
export const useRoute = () => useStore(route);

// ---------- ride session ----------
/** Undo window for an auto-applied (strong) coach call, ride seconds (DESIGN §7). */
export const UNDO_WINDOW_S = 15;
/** How long "NEO back · gap" stays in the status bar after a reconnect, ride seconds (DESIGN §5.2). */
export const RECONNECT_NOTICE_S = 10;

export interface SensorGap { sensor: "trainer" | "hrSensor"; lostAtT: number; backAtT: number | null }

export class RideSession {
  readonly workout: Workout;
  readonly status = createStore<RideStatus | null>(null);
  readonly nudges = createStore<CoachNudge[]>([]);
  readonly current = createStore<ReturnType<MockBackend["ride"]["current"]>>(null);
  readonly reverted = createStore<ReadonlySet<string>>(new Set());
  readonly gaps = createStore<SensorGap[]>([]);
  /** wall-clock ms when the rider paused (ride time stops while paused) */
  readonly pausedSinceMs = createStore<number | null>(null);
  /** the core's CapState / CapExceeded stream (§15.2): the episode in progress and the ride's log */
  readonly cap = createStore<{ over: CapExceeded | null; log: CapExceeded[] }>({ over: null, log: [] });
  private capMonitor: CapMonitor | null = null;
  private lastFedT = 0;
  /** once a trace is injected it stands in for measured power: later mock samples don't overwrite it */
  private injected = false;
  private readonly offs: (() => void)[] = [];
  private currentKey = "";

  constructor(workout: Workout) {
    this.workout = workout;
    backend.ride.load(workout);
    this.offs.push(
      backend.onStatus((s) => {
        this.trackGaps(s);
        if (s.state === "paused" && this.pausedSinceMs.get() === null) this.pausedSinceMs.set(performance.now());
        if (s.state !== "paused") this.pausedSinceMs.set(null);
        this.status.set(s);
        this.refreshCurrent();
        if (s.state === "finished" && route.get().view === "ride") navigate("review");
      }),
      backend.subscribe("power_3s", { rateHz: 4 }, (s) => this.injected || this.feedPower(s.t, s.v)),
      backend.onNudge((n) => {
        this.nudges.set((prev) => {
          const i = prev.findIndex((p) => p.callId === n.callId);
          return i < 0 ? [...prev, n] : prev.map((p, j) => (j === i ? n : p));
        });
        this.refreshCurrent();
      }),
    );
  }

  /** ride.current() deep-copies; refresh only when something that changes it changed. */
  private refreshCurrent() {
    const s = this.status.get();
    const key = `${s?.state}|${s?.blockIndex}|${s?.manualOffsetPct}|${this.nudges.get().map((n) => n.status).join()}`;
    if (key === this.currentKey) return;
    this.currentKey = key;
    this.current.set(backend.ride.current());
  }

  private trackGaps(s: RideStatus) {
    for (const sensor of ["trainer", "hrSensor"] as const) {
      const open = this.gaps.get().find((g) => g.sensor === sensor && g.backAtT === null);
      if (s[sensor] !== "connected" && !open) this.gaps.set((g) => [...g, { sensor, lostAtT: s.t, backAtT: null }]);
      if (s[sensor] === "connected" && open) this.gaps.set((g) => g.map((x) => (x === open ? { ...x, backAtT: s.t } : x)));
    }
  }

  /** Measured power → the cap monitor; an exceedance raises a strong ease_off once per episode (§15.2 point 3). */
  feedPower(t: number, w: number | null) {
    this.lastFedT = Math.max(this.lastFedT, t);
    if (!sceneFixture.get().healthCap) return;
    this.capMonitor ??= new CapMonitor(Math.round((this.workout.ftp * CAP_PCT_CP.threshold) / 100));
    const wasOver = this.cap.get().over !== null;
    const e = this.capMonitor.feed(t, w);
    this.cap.set({ over: e ? { ...e } : null, log: this.capMonitor.log.map((x) => ({ ...x })) });
    if (e && !wasOver)
      this.nudges.set((prev) => [...prev, { callId: `cap-${e.at}`, t, verdict: "ease_off", band: "strong", p: 1, message: "Above your injury cap", proposedPctChange: -3, status: "pending" }]);
  }

  /** Scenes and the safety test: a measured-power trace ([watts, seconds] steps at 4 Hz), as the core would stream it. */
  injectPower(trace: [number, number][]) {
    this.injected = true;
    let t = Math.max(this.status.get()?.t ?? 0, this.lastFedT);
    let last = 0;
    for (const [w, s] of trace)
      for (let i = 0; i < s * 4; i++) {
        t += 0.25;
        last = w;
        this.feedPower(t, w);
      }
    const c = this.current.get();
    const target = c?.compiled.blocks[this.status.get()?.blockIndex ?? 0]?.powerFrom ?? last;
    sceneFixture.set((f) => ({ ...f, metrics: { ...f.metrics, power_3s: last, delta_target: Math.round(last - target) } }));
  }

  markReverted(callId: string) {
    this.reverted.set((r) => new Set([...r, callId]));
  }

  dispose() {
    for (const off of this.offs) off();
  }
}

/**
 * What the core will put in RideStatus (ADR-0001 §5, ADR-0002 §6; #29), derived here from the mock's
 * status and this session's gap log, with scene fixtures layered on top.
 */
export function coreFacts(s: RideSession, status: RideStatus, nowMs: number): CoreFacts {
  const gaps = s.gaps.get();
  const tr = gaps.findLast((g) => g.sensor === "trainer");
  const open = tr && tr.backAtT === null ? tr : undefined;
  const lostForS = open ? status.t - open.lostAtT : 0;
  const paused = s.pausedSinceMs.get();
  const fx = sceneFixture.get().facts ?? {};
  return {
    trainerLost: !!open && lostForS >= DETECT_S,
    hrLost: status.hrSensor !== "connected",
    lostForS,
    reconnectAttempt: reconnectAttempt(lostForS),
    sinceReconnectS: tr && tr.backAtT !== null ? status.t - tr.backAtT : null,
    lastGapS: tr && tr.backAtT !== null ? tr.backAtT - tr.lostAtT : null,
    ergReleased: false,
    pausedForS: paused === null ? null : (nowMs - paused) / 1000,
    ...fx,
  };
}

export const session = createStore<RideSession | null>(null);
/** Stand-in status store for views rendered without a session. */
export const noStatus = createStore<RideStatus | null>(null);

/** Load a workout into a fresh session (the previous one is disposed: nothing carries over). */
export function loadRide(w: Workout): RideSession {
  session.get()?.dispose();
  const s = new RideSession(w);
  session.set(s);
  return s;
}

export const useSession = () => useStore(session);

// ---------- live metrics ----------
/** Latest sample of one metric at `rateHz` (wall clock); re-renders only the calling component. */
export function useMetric(metric: MetricId, rateHz: number): number | null {
  const [sample, setSample] = useState<Sample | null>(null);
  useEffect(() => backend.subscribe(metric, { rateHz }, setSample), [metric, rateHz]);
  const fixed = useStore(sceneFixture).metrics; // scene fixture: what the core would send in this state
  if (fixed && metric in fixed) return fixed[metric] ?? null;
  return sample?.v ?? null;
}

/** Re-run `read` at `hz` (polling budget, shared/README) and on mount. */
export function usePoll<T>(hz: number, read: () => T, deps: readonly unknown[]): T {
  const [value, setValue] = useState(read);
  useEffect(() => {
    setValue(read());
    if (sceneId) return; // scenes are frozen: one read is the truth
    const id = setInterval(() => {
      setValue(read());
    }, 1000 / hz);
    return () => {
      clearInterval(id);
    };
  }, deps);
  return value;
}

/** Planner calls resolve on the mock's clock; in a frozen scene, run that clock so they settle. */
export function settle<T>(p: Promise<T>): Promise<T> {
  if (sceneId) backend.advanceReal(1100);
  return p;
}
