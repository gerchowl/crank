// Contract stand-ins. The design needs data the contract (shared/src/contract.ts) does not offer
// yet; this is the ONLY place the prototype fakes or derives it, so no view computes physiology
// (DESIGN §10 #2). Each export names the issue that turns it into a real contract addition; the
// production UI must not import this module.
import type { Backend, CoachNudge, CompiledWorkout, FlatBlock, HrRecoveryState, RideStatus, StepNode, Workout } from "@crank/shared";
import { findNode, Rng } from "@crank/shared";

/** #13: the target band (± % of target) belongs in the contract. */
const TARGET_BAND_PCT = 5;
export const targetBand = (targetW: number) => ({ lo: targetW * (1 - TARGET_BAND_PCT / 100), hi: targetW * (1 + TARGET_BAND_PCT / 100) });

/** #13: τ "slower than usual" flag. */
export const tauSlower = (r: HrRecoveryState) => (r.fit ? r.fit.tauS >= 1.15 * r.fit.usualTauS : false);

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

/** #29: `coach.revert(callId)`. Stand-in: undo the applied % through the manual offset. */
export function revertNudge(backend: Backend, n: CoachNudge) {
  if (n.proposedPctChange) backend.ride.adjust(-n.proposedPctChange);
}

/** #29: sensor-loss policy thresholds come from the core (DESIGN §6.5). */
export const TRAINER_BLIP_S = 10;
export const trainerPolicy = (lostForS: number) => (lostForS < TRAINER_BLIP_S ? "Plan continues · ERG resumes on reconnect" : "Plan held · reconnecting");

/** #29: whether the ride shows a status as live or not. */
export const sensorOk = (s: RideStatus["trainer"]) => s === "connected";

// ---------- athlete model with uncertainty (#18) ----------
export interface Estimate { label: string; value: number; sd: number; unit: string; source: "prior" | "self_report" | "imported" | "test" | "rides"; n: number; updated: string }
export const athleteModel: Estimate[] = [
  { label: "Critical power", value: 280, sd: 12, unit: "W", source: "rides", n: 9, updated: "2026-10-11" },
  { label: "W′", value: 20.0, sd: 2.6, unit: "kJ", source: "rides", n: 9, updated: "2026-10-11" },
  { label: "HR max", value: 186, sd: 3, unit: "bpm", source: "rides", n: 9, updated: "2026-10-04" },
  { label: "HR-recovery τ", value: 41, sd: 6, unit: "s", source: "rides", n: 14, updated: "2026-10-11" },
  { label: "Durability", value: 93, sd: 4, unit: "% CP @ 2000 kJ", source: "prior", n: 0, updated: "2026-09-20" },
];
export const calibration = { confidence: "medium" as "low" | "medium" | "high", rides: 9, unlocksNext: "HR-vs-typical at 300 W after 20 more minutes there" };
/** Onboarding result: priors from the questionnaire (low confidence). */
export const priorModel: Estimate[] = [
  { label: "Critical power", value: 255, sd: 35, unit: "W", source: "self_report", n: 0, updated: "today" },
  { label: "W′", value: 18, sd: 5, unit: "kJ", source: "prior", n: 0, updated: "today" },
  { label: "HR max", value: 184, sd: 8, unit: "bpm", source: "prior", n: 0, updated: "today" },
];
/** CP trend (#18), oldest first. */
export const cpTrend = [262, 264, 263, 268, 271, 270, 274, 276, 280].map((v, i) => ({ week: `W${i + 33}`, cp: v, sd: 26 - i * 1.6 }));

// ---------- ride library (#29) ----------
export interface RideRow { id: string; date: string; name: string; minutes: number; tss: number; rpe: number; dsl: string }
export const pastRides: RideRow[] = [
  { id: "r9", date: "Sun 11 Oct", name: "Endurance 90′", minutes: 90, tss: 74, rpe: 4, dsl: `"Warm-up" 10m ramp 45-75%\n5x(12m 65%, 3m 70%)\n"Cool-down" 5m ramp 60-40%` },
  { id: "r8", date: "Fri 9 Oct", name: "Over-unders 3×9′", minutes: 55, tss: 68, rpe: 7, dsl: `"Warm-up" 10m ramp 45-75%\n3x[!last](3x(2m 95%, 1m 108%), 4m 50%)\n"Cool-down" 7m ramp 60-40%` },
  { id: "r7", date: "Wed 7 Oct", name: "Recovery spin 40′", minutes: 40, tss: 22, rpe: 2, dsl: `40m 50%` },
  { id: "r6", date: "Tue 6 Oct", name: "4×4′ VO2max", minutes: 50, tss: 61, rpe: 8, dsl: `"Warm-up" 10m ramp 45-75%\n4x[!last](4m 110%, 3m 50%)\n"Cool-down" 7m ramp 60-40%` },
  { id: "r5", date: "Sun 4 Oct", name: "Endurance 2h", minutes: 120, tss: 96, rpe: 5, dsl: `"Warm-up" 10m ramp 45-75%\n7x(12m 65%, 3m 70%)\n"Cool-down" 5m ramp 60-40%` },
];

/** Last 7 days (#29): day, load done/planned, kind. */
export const week = [
  { day: "Wed", tss: 22, kind: "done" },
  { day: "Thu", tss: 0, kind: "rest" },
  { day: "Fri", tss: 68, kind: "done" },
  { day: "Sat", tss: 0, kind: "rest" },
  { day: "Sun", tss: 74, kind: "done" },
  { day: "Mon", tss: 0, kind: "rest" },
  { day: "Tue", tss: 0, kind: "today" },
] as const;

// ---------- per-lap stats (#12) ----------
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

// ---------- devices (#29) ----------
export interface Device { id: string; role: "Trainer" | "Heart rate"; name: string; protocol: string; battery: number | null; state: "connected" | "available" | "remembered" }
export const devices: Device[] = [
  { id: "neo", role: "Trainer", name: "Tacx Neo 55624", protocol: "FE-C over BLE · fw 0.7.4", battery: null, state: "connected" },
  { id: "hrm", role: "Heart rate", name: "HRM-Pro 41822", protocol: "Heart Rate Service", battery: 78, state: "connected" },
  { id: "h10", role: "Heart rate", name: "Polar H10 8C21", protocol: "Heart Rate Service", battery: null, state: "available" },
];

/** Yesterday's check-in, pre-filled on Today (#29: persisted check-ins). */
export const lastCheckin = { sleep: 4, legs: 3, stress: 2, motivation: 4, minutes: 45 } as const;
