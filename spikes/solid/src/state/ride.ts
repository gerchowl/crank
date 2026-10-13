// App-lifetime ride state. RideStatus lives in a store updated with reconcile(): the 2 Hz
// heartbeat only changes `t`, so only computations that read `status.t` re-run — the block
// index, phase, connection dots etc. stay untouched.
import { batch, createEffect, createRoot, createSignal, on } from "solid-js";
import { createStore, produce, reconcile } from "solid-js/store";
import type { CoachNudge, CompiledWorkout, HrRecoveryState, MetricId, RideStatus, Workout } from "@neo/shared";
import { backend } from "../backend";

export type Speed = 1 | 10 | 60;

/** Per-ride recording for the review screen (contract gap: no full-ride linear stream). */
export interface Recording {
  t: number[];
  power: (number | null)[];
  hr: (number | null)[];
  cadence: (number | null)[];
  dfa: (number | null)[];
  wbal: (number | null)[];
  kj: (number | null)[];
  last: Partial<Record<MetricId, number>>;
  recoveries: HrRecoveryState[];
}

const RECORDED = ["power", "hr", "cadence", "dfa_a1", "wbal_pct", "kj"] as const;
const SUMMARY: readonly MetricId[] = ["np", "if", "tss", "kj", "decoupling", "compliance", "elapsed", "hr_drift"];

function emptyRecording(): Recording {
  return { t: [], power: [], hr: [], cadence: [], dfa: [], wbal: [], kj: [], last: {}, recoveries: [] };
}

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

  // ---------- recorder ----------
  let rec = emptyRecording();
  const [recording, setRecording] = createSignal<Recording>(rec, { equals: false });
  const lastV: Record<string, number | null> = {};
  for (const m of RECORDED) {
    backend.subscribe(m, { rateHz: 10 }, (s) => {
      lastV[m] = s.v;
      if (m !== "power" || s.v === null || s.t <= 0) return;
      const prev = rec.t[rec.t.length - 1];
      if (prev !== undefined && s.t <= prev) return;
      rec.t.push(s.t);
      rec.power.push(s.v);
      rec.hr.push(lastV["hr"] ?? null);
      rec.cadence.push(lastV["cadence"] ?? null);
      rec.dfa.push(lastV["dfa_a1"] ?? null);
      rec.wbal.push(lastV["wbal_pct"] ?? null);
      rec.kj.push(lastV["kj"] ?? null);
    });
  }
  for (const m of SUMMARY) {
    backend.subscribe(m, { rateHz: 1 }, (s) => {
      if (s.v !== null) rec.last[m] = s.v;
    });
  }
  // publish the recording to reactive readers at most once a second
  window.setInterval(() => {
    if (status.state === "riding") setRecording(rec);
  }, 1000);

  // HR-recovery blocks: keep the final fit of each one (poll once when the phase leaves recovery)
  createEffect(
    on(
      () => status.phase,
      (phase, prev) => {
        if (prev === "recovery" && phase !== "recovery") {
          const st = backend.hrRecovery();
          if (st.fit) rec.recoveries.push(st);
          setRecording(rec);
        }
      },
    ),
  );

  const resetRecording = () => {
    rec = emptyRecording();
    setRecording(rec);
  };

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
      resetRecording();
      setNudges([]);
      setCurrent(backend.ride.current());
    },
    stop() {
      backend.ride.stop();
      setRecording(rec);
    },
    respond(callId: string, accept: boolean) {
      backend.coach.respond(callId, accept);
    },
  };

  return { status, nudges, current, recording, controls };
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
