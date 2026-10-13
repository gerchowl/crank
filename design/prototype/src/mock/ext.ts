// Contract stand-ins. The design needs data the contract (shared/src/contract.ts) does not offer
// yet; this is the ONLY place the prototype fakes or derives it, so no view computes physiology
// (DESIGN §10 #2). Each export names the issue that turns it into a real contract addition; the
// production UI must not import this module. Scene fixtures (states the mock cannot produce) are
// at the bottom and are labelled as such.
import type { Backend, CoachNudge, CompiledWorkout, FlatBlock, HrRecoveryState, Proposal, RideStatus, StepNode, Workout } from "@crank/shared";
import { findNode, Rng } from "@crank/shared";
import { createStore } from "../store";

/** #13: the target band (± % of target) belongs in the contract. */
const TARGET_BAND_PCT = 5;
export const targetBand = (targetW: number) => ({ lo: targetW * (1 - TARGET_BAND_PCT / 100), hi: targetW * (1 + TARGET_BAND_PCT / 100) });

/** #13 / ADR-0002 §7.8: settle window after a target discontinuity; the marker and Δ are hidden meanwhile. */
export const SETTLE_S = 5;
export function settling(block: FlatBlock, prev: FlatBlock | undefined, blockElapsedS: number, sinceReconnectS: number | null) {
  if (sinceReconnectS !== null && sinceReconnectS < 2 * SETTLE_S) return true;
  const stepChange = !prev || Math.abs(prev.powerTo - block.powerFrom) > 1;
  return stepChange && blockElapsedS < SETTLE_S;
}

/** #13: cadence band state (the ERG lever). */
export type CadenceState = "in" | "below" | "above" | "none";
export function cadenceState(rpm: number | null, step: StepNode | undefined): CadenceState {
  if (!step?.cadence || rpm === null) return "none";
  if (rpm < step.cadence.lo - 2) return "below";
  if (rpm > step.cadence.hi + 5) return "above";
  return "in";
}

/** #13: τ "slower than usual" flag. */
export const tauSlower = (r: HrRecoveryState) => (r.fit ? r.fit.tauS >= 1.15 * r.fit.usualTauS : false);

/** #13: whether a summary metric means something for this session (decoupling needs a steady ride). */
export const metricValid = (metric: string, c: CompiledWorkout) => metric !== "decoupling" || !c.blocks.some((b) => b.phase === "work" && b.durS < 600);

/** #14: live cursor on the planned timeline (expected start of the block + time spent in it). */
export const cursorS = (block: FlatBlock, blockRemainingS: number | null) =>
  block.startS + Math.max(0, Math.min(block.durS, block.durS - (blockRemainingS ?? block.durS)));

/** #29: the core should tell the UI which ride template a block needs (DESIGN §5.2). */
export type BlockKind = "ramp" | "steady" | "onoff" | "sprint" | "hr_recovery" | "easy" | "cooldown";
export function blockKind(b: FlatBlock, step: StepNode | undefined, ftp: number): BlockKind {
  if (step?.end.kind === "hr_recovery") return "hr_recovery";
  if (b.phase === "cooldown") return "cooldown";
  if (b.powerFrom !== b.powerTo) return "ramp";
  if (b.phase === "work" && b.durS <= 30 && b.powerFrom >= 1.5 * ftp) return "sprint";
  if (b.phase === "work") return b.durS <= 60 ? "onoff" : "steady";
  return b.durS <= 60 ? "onoff" : "easy";
}

// ---------- coach (#29) ----------
/** `coach.revert(callId)`. Stand-in: undo the applied % through the manual offset. */
export function revertNudge(backend: Backend, n: CoachNudge) {
  if (n.proposedPctChange) backend.ride.adjust(-n.proposedPctChange);
}
/** Short reason (≤ 24 chars) for the lane; the full message stays for Review. */
export function coachReason(n: CoachNudge): string {
  const m = n.message;
  const hr = /HR is (\d+) bpm above/.exec(m) ?? /HR (\d+) bpm below/.exec(m);
  if (hr) return `HR ${n.verdict === "push" ? "−" : "+"}${hr[1]} bpm vs usual`;
  const cad = /Cadence fading \((\d+) rpm\)/.exec(m);
  if (cad) return `Cadence fading: ${cad[1]} rpm`;
  const tau = /τ (\d+) s vs usual (\d+) s/.exec(m);
  if (tau) return `Recovery slow: τ ${tau[1]} s`;
  if (/unreachable/.test(m)) return "HR target unreachable";
  return m.slice(0, 24);
}
/** The coach's cumulative change to the plan (the rider's own ± is separate). */
export const coachOffsetPct = (nudges: CoachNudge[], reverted: ReadonlySet<string>) =>
  nudges.filter((n) => (n.status === "auto_applied" || n.status === "accepted") && !reverted.has(n.callId)).reduce((a, n) => a + (n.proposedPctChange ?? 0), 0);

// ---------- ride status additions (ADR-0001 §5, ADR-0002 §6; #29) ----------
export const DETECT_S = 2;
const BACKOFF_S = [0.5, 1, 2, 4];
export interface CoreFacts {
  trainerLost: boolean; // past the 2 s detection: paused(trainer_lost)
  hrLost: boolean;
  lostForS: number;
  reconnectAttempt: number;
  /** seconds since the trainer came back (soft start = first 5 s), null if no recent reconnect */
  sinceReconnectS: number | null;
  lastGapS: number | null;
  ergReleased: boolean;
  pausedForS: number | null;
}
export function reconnectAttempt(lostForS: number) {
  let t = DETECT_S;
  let n = 1;
  for (const b of BACKOFF_S) {
    if (lostForS < t + b) return n;
    t += b;
    n++;
  }
  return n + Math.floor((lostForS - t) / 5);
}

// ---------- athlete model with uncertainty (#18) ----------
export interface Estimate { label: string; value: number; sd: number; unit: string; source: "prior" | "self_report" | "imported" | "test" | "rides"; n: number; updated: string }
export const athleteModel: Estimate[] = [
  { label: "Critical power", value: 280, sd: 12, unit: "W", source: "rides", n: 9, updated: "11 Oct" },
  { label: "W′", value: 20.0, sd: 2.6, unit: "kJ", source: "rides", n: 9, updated: "11 Oct" },
  { label: "HR max", value: 186, sd: 3, unit: "bpm", source: "rides", n: 9, updated: "4 Oct" },
  { label: "HR-recovery τ", value: 41, sd: 6, unit: "s", source: "rides", n: 14, updated: "11 Oct" },
  { label: "Durability", value: 93, sd: 4, unit: "% CP @ 2000 kJ", source: "prior", n: 0, updated: "20 Sep" },
];
export const calibration = { confidence: "medium" as "low" | "medium" | "high", rides: 9, unlocksNext: "HR-vs-usual at 300 W after 20 more minutes there" };
/** Onboarding result: priors from the questionnaire (low confidence). */
export const priorModel: Estimate[] = [
  { label: "Critical power", value: 255, sd: 35, unit: "W", source: "self_report", n: 0, updated: "today" },
  { label: "W′", value: 18, sd: 5, unit: "kJ", source: "prior", n: 0, updated: "today" },
  { label: "HR max", value: 184, sd: 8, unit: "bpm", source: "prior", n: 0, updated: "today" },
];
/** CP trend (#18), oldest first: week starting date, value, sd. */
export const cpTrend = ["17 Aug", "24 Aug", "31 Aug", "7 Sep", "14 Sep", "21 Sep", "28 Sep", "5 Oct", "12 Oct"].map((week, i) => ({
  week,
  cp: [262, 264, 263, 268, 271, 270, 274, 276, 280][i] ?? 0,
  sd: Math.round(26 - i * 1.6),
}));

// ---------- planner (#15, #29) ----------
/** Structured, checkable reasons for a proposal (the planner returns prose today). */
export function proposalReasons(p: Proposal, minutes: number, legs: string, sleep: string): string[] {
  return [
    `You said: legs ${legs}, sleep ${sleep}.`,
    `Last threshold work: over-unders on Fri 9 Oct, 4 days ago.`,
    `Load over 7 days: 164 TSS — below your usual 210, so there is room.`,
    `Fits your ${minutes} min: ${p.workout.name}.`,
  ];
}

// ---------- ride library (#29) ----------
export interface RideRow { id: string; date: string; name: string; seconds: number; tss: number; rpe: number; dsl: string }
export const pastRides: RideRow[] = [
  { id: "r9", date: "Sun 11 Oct", name: "Endurance 90′", seconds: 5400, tss: 74, rpe: 4, dsl: `"Warm-up" 10m ramp 45-75%\n5x(12m 65%, 3m 70%)\n"Cool-down" 5m ramp 60-40%` },
  { id: "r8", date: "Fri 9 Oct", name: "Over-unders 3×9′", seconds: 3300, tss: 68, rpe: 7, dsl: `"Warm-up" 10m ramp 45-75%\n3x[!last](3x(2m 95%, 1m 108%), 4m 50%)\n"Cool-down" 7m ramp 60-40%` },
  { id: "r7", date: "Wed 7 Oct", name: "Recovery spin 40′", seconds: 2400, tss: 22, rpe: 2, dsl: `40m 50%` },
  { id: "r6", date: "Tue 6 Oct", name: "4×4′ VO2max", seconds: 3000, tss: 61, rpe: 8, dsl: `"Warm-up" 10m ramp 45-75%\n4x[!last](4m 110%, 3m 50%)\n"Cool-down" 7m ramp 60-40%` },
  { id: "r5", date: "Sun 4 Oct", name: "Endurance 2h", seconds: 7200, tss: 96, rpe: 5, dsl: `"Warm-up" 10m ramp 45-75%\n7x(12m 65%, 3m 70%)\n"Cool-down" 5m ramp 60-40%` },
];

/** Last 7 days (#29): day, load done, kind. */
export const week = [
  { day: "Wed", tss: 22, kind: "done" },
  { day: "Thu", tss: 0, kind: "rest" },
  { day: "Fri", tss: 68, kind: "done" },
  { day: "Sat", tss: 0, kind: "rest" },
  { day: "Sun", tss: 74, kind: "done" },
  { day: "Mon", tss: 0, kind: "rest" },
  { day: "Tue", tss: 0, kind: "today" },
] as const;

// ---------- per-lap stats and intent verdict (#12) ----------
export interface LapRow { label: string; targetW: number; avgW: number; avgHr: number; cadence: number; compliancePct: number }
/** Fixture: plausible per-interval numbers for the work blocks of a compiled ride. */
export function workIntervals(w: Workout, c: CompiledWorkout): LapRow[] {
  const rng = new Rng(7);
  return c.blocks
    .filter((b) => b.phase === "work" && b.durS >= 60)
    .map((b, i) => {
      const node = findNode(w.nodes, b.nodeId);
      const target = Math.round((b.powerFrom + b.powerTo) / 2);
      return {
        label: `${node?.label ?? "Work"} ${i + 1}`,
        targetW: target,
        avgW: Math.round(target * (1 + rng.uniform(-0.02, 0.01))),
        avgHr: Math.round(158 + i * 3 + rng.uniform(-2, 2)),
        cadence: Math.round(rng.uniform(88, 94)),
        compliancePct: Math.round(rng.uniform(88, 99)),
      };
    });
}
/** One line per part of the workout: did it do its job (the core computes this from lap stats). */
export function intentVerdict(c: CompiledWorkout, recoveries: HrRecoveryState[]): { part: string; verdict: string; ok: boolean }[] {
  const shortReps = c.blocks.filter((b) => b.phase === "work" && b.durS < 60).length;
  const slow = recoveries.findIndex(tauSlower);
  const out = [];
  if (shortReps) out.push({ part: "VO2 sets", verdict: `${shortReps} of ${shortReps} reps ridden on target`, ok: true });
  out.push({ part: "Threshold 2 × 4:00", verdict: "97 % of target on average — as planned", ok: true });
  if (recoveries.length)
    out.push({ part: "HR recoveries", verdict: slow < 0 ? `${recoveries.length} of ${recoveries.length} within your usual τ` : `recovery ${slow + 1} slower than usual`, ok: slow < 0 });
  return out;
}

// ---------- devices (#29) ----------
export interface Device { id: string; role: "Trainer" | "Heart rate"; name: string; protocol: string; battery: number | null; state: "connected" | "available" | "remembered"; live: string | null }
export const devices: Device[] = [
  { id: "neo", role: "Trainer", name: "Tacx Neo 55624", protocol: "FE-C over BLE · fw 0.7.4", battery: null, state: "connected", live: "112 W · 84 rpm" },
  { id: "hrm", role: "Heart rate", name: "HRM-Pro 41822", protocol: "Heart Rate Service", battery: 78, state: "connected", live: "96 bpm" },
  { id: "h10", role: "Heart rate", name: "Polar H10 8C21", protocol: "Heart Rate Service", battery: null, state: "available", live: null },
];

/** Yesterday's check-in, pre-filled on Today (#29: persisted check-ins). */
export const lastCheckin = { sleep: 4, legs: 3, stress: 2, motivation: 4, minutes: 45 } as const;

/** Interrupted ride found on launch (#50: resumable rides). */
export const interruptedRide = { workout: "Demo: VO2 sets + threshold (45′)", stoppedAt: "19:40 yesterday", riddenS: 1872, savedS: 1867, block: "block 36 of 39 · Threshold rep 1 of 2", gapNote: "crank saves every 5 s" };

// ---------- scene fixtures (states the mock cannot produce; DESIGN §12) ----------
export interface SceneFixture {
  status?: Partial<RideStatus>;
  facts?: Partial<CoreFacts>;
  screenCrashed?: boolean;
  feelPrompt?: boolean;
  healthCap?: boolean;
  /** Review: expand the coach call with this index */
  openCall?: number;
}
export const sceneFixture = createStore<SceneFixture>({});
