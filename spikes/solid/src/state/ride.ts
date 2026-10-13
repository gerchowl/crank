// App-lifetime ride state. RideStatus lives in a store updated with reconcile(): the 2 Hz
// heartbeat only changes `t`, so only computations that read `status.t` re-run — the block
// index, phase, connection dots etc. stay untouched.
import { batch, createEffect, createRoot, createSignal, on } from "solid-js";
import { createStore, produce, reconcile } from "solid-js/store";
import type { CoachNudge, CompiledWorkout, RideStatus, Workout } from "@neo/shared";
import { backend } from "../backend";

export type Speed = 1 | 10 | 60;

export const ride = createRoot(() => {
  const [status, setStatus] = createStore<RideStatus>({
    state: "idle",
    t: 0,
    speed: 10,
    mode: "erg",
    trainer: "connecting",
    hrSensor: "connecting",
    blockIndex: 0,
    phase: "warmup",
    manualOffsetPct: 0,
  });
  backend.onStatus((s) => setStatus(reconcile(s)));

  // nudges by callId (store → each card only updates its own fields)
  const [nudges, setNudges] = createStore<CoachNudge[]>([]);
  const [nudgeVersion, setNudgeVersion] = createSignal(0);
  backend.onNudge((n) => {
    batch(() => {
      setNudges(
        produce((list) => {
          const i = list.findIndex((x) => x.callId === n.callId);
          if (i < 0) list.push(n);
          else Object.assign(list[i] ?? {}, n);
        }),
      );
      setNudgeVersion((v) => v + 1);
    });
  });

  // current workout + compiled blocks: refreshed on discrete changes only (polling budget)
  const [current, setCurrent] = createSignal<{ workout: Workout; compiled: CompiledWorkout } | null>(backend.ride.current());
  createEffect(
    on([() => status.blockIndex, () => status.state, () => status.manualOffsetPct, nudgeVersion], () => {
      setCurrent(backend.ride.current());
    }),
  );

  // ---------- controls ----------
  const controls = {
    togglePause() {
      if (status.state === "riding") backend.ride.pause();
      else if (status.state === "paused") backend.ride.resume();
      else if (status.state === "idle") backend.ride.start();
    },
    start() {
      backend.ride.start();
    },
    adjust(pct: number) {
      backend.ride.adjust(pct);
    },
    skip() {
      backend.ride.skipBlock();
    },
    setSpeed(s: Speed) {
      backend.ride.setSpeed(s);
    },
    load(w: Workout) {
      backend.ride.load(w);
      setNudges([]);
      setCurrent(backend.ride.current());
    },
    stop() {
      backend.ride.stop();
    },
    respond(callId: string, accept: boolean) {
      backend.coach.respond(callId, accept);
    },
  };

  return { status, nudges, current, controls };
});

/** The pending nudge to show (newest pending, else newest of any status). */
export function latestNudge(): CoachNudge | undefined {
  const list = ride.nudges;
  for (let i = list.length - 1; i >= 0; i--) {
    const n = list[i];
    if (n?.status === "pending") return n;
  }
  return list[list.length - 1];
}
