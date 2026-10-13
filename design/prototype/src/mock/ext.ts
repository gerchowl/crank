// Contract stand-ins. The design needs data the contract (shared/src/contract.ts) does not offer
// yet; this is the ONLY place the prototype fakes or derives it, so no view computes physiology
// (DESIGN §10 #2). Each export names the issue that turns it into a real contract addition; the
// production UI must not import this module. Scene fixtures (states the mock cannot produce) are
// at the bottom and are labelled as such.
import type { Backend, CoachNudge, CompiledWorkout, FlatBlock, HrRecoveryState, MetricId, Proposal, RideStatus, StepNode, Workout } from "@crank/shared";
import { findNode, parseDsl, Rng } from "@crank/shared";
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

/** #13: τ "slower than usual" flag; the usual τ carries its spread so "usual" is a range, not a point. */
export const TAU_SLOWER_FACTOR = 1.15;
export const tauSlower = (r: HrRecoveryState) => (r.fit ? r.fit.tauS >= TAU_SLOWER_FACTOR * r.fit.usualTauS : false);
export const USUAL_TAU_SD_S = 6;
/** #29 (fit quality): τ uncertainty from how much recovery the fit saw (stand-in until the core reports it). */
export function tauSd(r: HrRecoveryState): number | null {
  if (!r.fit) return null;
  const minutes = Math.max(0.5, ((r.observed.at(-1)?.t ?? r.startedT) - r.peakT) / 60);
  return Math.round(r.fit.tauS * (0.25 / Math.sqrt(minutes)));
}

/**
 * #13 / #29: what the recovery estimate means. readyInS is clamped to the block's min/max, so a band
 * pinned to a limit is not a prediction of HR crossing — it is the block's rule.
 */
export type RecoveryRead =
  | { kind: "fitting" }
  | { kind: "ends_at_min"; inS: number } // HR is predicted below target before the minimum
  | { kind: "estimate"; lo: number; hi: number; mean: number; capped: boolean } // capped = hi is the block maximum
  | { kind: "hr_lost"; inS: number };
export function recoveryRead(r: HrRecoveryState, elapsedS: number, hrLost: boolean): RecoveryRead {
  const toMin = Math.max(0, r.minS - elapsedS);
  const toMax = Math.max(0, r.maxS - elapsedS);
  if (hrLost) return { kind: "hr_lost", inS: toMax };
  const ready = r.readyInS;
  if (!ready) return { kind: "fitting" };
  if (ready.hi <= toMin + 1) return { kind: "ends_at_min", inS: toMin };
  return { kind: "estimate", lo: ready.lo, hi: ready.hi, mean: ready.mean, capped: ready.hi >= toMax - 1 };
}

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
  // ages in days on 13 Oct: Today and History show how old each estimate is (stale ≥ 42 d)
  { label: "HR-recovery τ", value: 55, sd: 6, unit: "s", source: "rides", n: 14, updated: "11 Oct" },
  { label: "Durability", value: 93, sd: 4, unit: "% CP @ 2000 kJ", source: "prior", n: 0, updated: "20 Sep" },
];
export const calibration = { confidence: "medium" as "low" | "medium" | "high", rides: 9, unlocksNext: "HR-vs-usual at 300 W after 20 more minutes there" };
/** Days between an "11 Oct"-style date and today (13 Oct 2026); estimates ≥ STALE_DAYS old look stale. */
export const STALE_DAYS = 42;
export function ageDays(d: string): number {
  const months: Record<string, number> = { Aug: 7, Sep: 8, Oct: 9 };
  const [day, mon] = d.split(" ");
  const t = Date.UTC(2026, months[mon ?? ""] ?? 9, Number(day));
  return Math.round((Date.UTC(2026, 9, 13) - t) / 86400000);
}
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
export function proposalReasons(p: Proposal, minutes: number, legs: string, sleep: string, capped = false): string[] {
  if (capped)
    return [
      `Capped by your health answers: endurance only (≤ ${CAP_PCT.endurance} % FTP) until a medical check is confirmed.`,
      `You said: legs ${legs}, sleep ${sleep} — within the cap that means steady aerobic riding.`,
      `Fits your ${minutes} min: ${p.workout.name}.`,
    ];
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
/** "On target" has one definition everywhere (ADR-0002 §7.8): share of ERG seconds within ±5 % of the controlled target, settle excluded. */
export const ON_TARGET_DEF = "on target = share of ERG seconds within ±5 % of the target you were given, settling excluded";
export function intentVerdict(c: CompiledWorkout, recoveries: HrRecoveryState[], laps: LapRow[], coachPct: number): { part: string; verdict: string; ok: boolean }[] {
  const shortReps = c.blocks.filter((b) => b.phase === "work" && b.durS < 60).length;
  const slow = recoveries.findIndex(tauSlower);
  const out = [];
  if (shortReps) out.push({ part: "VO2 sets", verdict: `${shortReps} of ${shortReps} reps ridden on target`, ok: true });
  const onTarget = laps.length ? Math.round(laps.reduce((a, l) => a + l.compliancePct, 0) / laps.length) : null;
  if (onTarget !== null)
    out.push({ part: `Threshold ${laps.length} × 4:00`, verdict: `${onTarget} % on target${coachPct ? `, against the eased target (plan ${coachPct > 0 ? "+" : "−"}${Math.abs(coachPct)} % by coach)` : ""}`, ok: onTarget >= 85 });
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

/** Interrupted ride found on launch (#50: resumable rides); `agoS` decides the re-entry plan. */
export const interruptedRide = { workout: "Demo: VO2 sets + threshold (45′)", riddenS: 1872, savedS: 1867, block: "block 36 of 39 · Threshold rep 1 of 2, 1:40 into it", inWork: true, gapNote: "crank saves every 5 s" };

// ---------- typical HR (#29: a "usual HR for this power" stream) ----------
/** Usual HR = HR minus the backend's hr_vs_typical residual (stand-in for a typical-HR stream). */
export const usualHr = (hr: (number | null)[], vsTyp: (number | null)[]) => hr.map((h, i) => (h === null || vsTyp[i] == null ? null : h - (vsTyp[i] as number)));

// ---------- health check (#29: health-cap state; DESIGN §5.8) ----------
export type HealthAnswer = "yes" | "no" | null;
export const HEALTH_QS = [
  { id: "chest", q: "Chest pain, or a heart condition, on exertion?", ifYes: "stop" },
  { id: "faint", q: "Dizziness or fainting during or after exercise?", ifYes: "stop" },
  { id: "meds", q: "Medication that changes your heart rate (e.g. beta blockers)?", ifYes: "hr" },
  { id: "injury", q: "An injury that limits hard efforts?", ifYes: "cap" },
] as const;
export type HealthId = (typeof HEALTH_QS)[number]["id"];
/** Consequences differ by answer: stop-and-check, HR features off, or an intensity cap. */
export function healthConsequences(a: Partial<Record<HealthId, HealthAnswer>>) {
  const yes = (id: HealthId) => a[id] === "yes";
  return {
    complete: HEALTH_QS.every((h) => a[h.id] === "yes" || a[h.id] === "no"),
    stop: yes("chest") || yes("faint"),
    hrOff: yes("meds"),
    cap: yes("chest") || yes("faint") ? "endurance" : yes("injury") ? "threshold" : null,
  } as const;
}
/** The planner respects the cap (stand-in: a capped workout instead of the mock's uncapped proposal). */
export const CAP_PCT = { endurance: 75, threshold: 100 } as const;
export function cappedProposal(ftp: number, minutes: number): Proposal {
  const main = Math.max(10, minutes - 15);
  const r = parseDsl(`"Warm-up" 10m ramp 45-70% 90rpm
"Endurance" ${main}m 68% 85-95rpm
"Cool-down" 5m ramp 60-40%`, ftp);
  if ("error" in r) throw new Error(r.error.message);
  return { workout: { id: "capped", name: `Endurance ${minutes}′ (capped)`, ftp, nodes: r.nodes }, rationale: "Capped at endurance until the medical check is confirmed." };
}

// ---------- interrupted rides (#50; DESIGN §6.6) ----------
/** Re-entry policy keyed to how long ago the ride stopped and where. */
export const RESUME = { softOnlyS: 120, rewarmS: 15 * 60, rewarmBlockS: 180 } as const;
export type ResumePlan = { kind: "continue"; note: string } | { kind: "rewarm"; note: string } | { kind: "stale"; note: string };
export function resumePlan(stoppedAgoS: number, inWork: boolean): ResumePlan {
  if (stoppedAgoS <= RESUME.softOnlyS) return { kind: "continue", note: "continues where you stopped; ERG eases in over 5 s" };
  if (stoppedAgoS <= RESUME.rewarmS)
    return { kind: "rewarm", note: `adds a ${RESUME.rewarmBlockS / 60}-min re-warm-up ramp${inWork ? ", then restarts the interval you were in from its start" : ""}` };
  return { kind: "stale", note: "too long ago to pick up the same session" };
}

// ---------- plain-language edits (#29: planner rewrite rationale) ----------
export function humanToken(t: string): string {
  let m: RegExpExecArray | null;
  if ((m = /^(\d+)x/.exec(t))) return `${m[1]} times`;
  if ((m = /^(\d+(?:\.\d+)?)%$/.exec(t))) return `${m[1]} % FTP`;
  if ((m = /^(\d+)-(\d+)%$/.exec(t))) return `${m[1]}–${m[2]} % FTP`;
  if ((m = /^hr<(\d+)$/.exec(t))) return `until HR < ${m[1]}`;
  if ((m = /^(\d+)rpm$/.exec(t))) return `${m[1]} rpm`;
  if ((m = /^(?:(\d+)m)?(?:(\d+)s)?$/.exec(t)) && (m[1] || m[2])) return `${m[1] ? `${m[1]} min` : ""}${m[1] && m[2] ? " " : ""}${m[2] ? `${m[2]} s` : ""}`;
  return t;
}

// ---------- scene fixtures (states the mock cannot produce; DESIGN §12) ----------
export interface SceneFixture {
  /** live metric values the core would send in this state (e.g. cadence < 40 rpm for ERG release) */
  metrics?: Partial<Record<MetricId, number | null>>;
  status?: Partial<RideStatus>;
  facts?: Partial<CoreFacts>;
  screenCrashed?: boolean;
  feelPrompt?: boolean;
  healthCap?: boolean;
  /** Review: expand the coach call with this index */
  openCall?: number;
  /** Resume sheet: seconds since the interrupted ride stopped */
  stoppedAgoS?: number;
  /** Onboarding: health answers so far */
  health?: Partial<Record<HealthId, HealthAnswer>>;
}
export const sceneFixture = createStore<SceneFixture>({});
