// Contract stand-ins for the training profile (#85) and the planner wish (#86): DESIGN §15. The planner
// decides how a wish is treated and says why (contract fields for #59, DESIGN §15.4); this module fakes
// that decision for the prototype, like ext.ts. The production UI must not import it.
import { parseDsl, type Proposal } from "@crank/shared";
import type { Grade } from "../knowledge";
import { CAP_RULE } from "./ext";

/** Intensity of the main effort, low → high, anchored on the athlete's thresholds (R-GEN-01), not zone numbers. */
export const INTENSITIES = ["easy", "tempo", "threshold", "vo2max", "sprint"] as const;
/** How the main effort is structured. Duration is a separate choice (Today's time), never a format. */
export const FORMATS = ["steady", "intervals", "hills", "race", "open"] as const;
export type Intensity = (typeof INTENSITIES)[number];
export type Format = (typeof FORMATS)[number];
export interface Cell { intensity: Intensity; format: Format }
export type Wish = Cell | null; // null = coach decides

export const INTENSITY_NAME: Record<Intensity, [string, string]> = {
  easy: ["Easy", "below VT1"],
  tempo: ["Tempo", "VT1 to below CP"],
  threshold: ["Threshold", "at CP"],
  vo2max: ["VO2max", "above CP"],
  sprint: ["Sprint", "all-out ≤ 30 s"],
};
export const FORMAT_NAME: Record<Format, string> = { steady: "Steady", intervals: "Intervals", hills: "Hills", race: "Race sim", open: "Open ride" };
/** Trainer mode per format. Only ERG bounds the rider's power (DESIGN §15.2 health cap). */
export const FORMAT_MODE: Record<Format, "ERG" | "SIM" | "free"> = { steady: "ERG", intervals: "ERG", hills: "SIM", race: "ERG", open: "free" };
export const cellName = (c: Cell) => `${INTENSITY_NAME[c.intensity][0]} · ${FORMAT_NAME[c.format]}`;
export const sameCell = (a: Cell | null | undefined, b: Cell | null | undefined) => !!a && !!b && a.intensity === b.intensity && a.format === b.format;
const rank = (i: Intensity) => INTENSITIES.indexOf(i);
const n = (x: number) => Math.max(1, Math.floor(x));

/** The session family behind each coherent cell, sized to the main-set minutes; cells not listed describe no session. */
const SESSIONS: Partial<Record<`${Intensity}.${Format}`, (m: number) => [string, string]>> = {
  "easy.steady": (m) => [`Endurance`, `"Endurance" ${m}m 65%`],
  "easy.intervals": (m) => [`Easy with cadence drills`, `"Drills" ${n(m / 6)}x(5m 62%, 1m 65% 105-115rpm)`],
  "easy.hills": (m) => [`Rolling easy`, `"Rolling" ${n(m / 6)}x(4m 60%, 2m 72% 75-85rpm)`],
  "easy.open": (m) => [`Open ride, easy ceiling`, `"Open" ${m}m free`],
  "tempo.steady": (m) => [`Steady tempo`, `"Endurance" ${Math.max(5, m - Math.min(60, m - 5))}m 65%\n"Tempo" ${Math.min(60, m - 5)}m 82%`],
  "tempo.intervals": (m) => [`Tempo ${n((m + 5) / 15)}×10′`, `"Tempo" ${n((m + 5) / 15)}x[!last](10m 85% 85-95rpm, 5m 55%)`],
  "tempo.hills": (m) => [`Tempo climbs ${n((m + 4) / 12)}×8′`, `"Climb" ${n((m + 4) / 12)}x[!last](8m 85% 70-80rpm, 4m 55%)`],
  "tempo.race": (m) => [`Group-ride sim`, `"Race" ${n(m / 5)}x(4m 78%, 1m 105%)`],
  "tempo.open": (m) => [`Open ride, tempo ceiling`, `"Open" ${m}m free`],
  "threshold.steady": (m) => [`Steady threshold ${Math.min(30, m - 5)}′`, `"Endurance" ${Math.max(5, m - Math.min(30, m - 5))}m 65%\n"Threshold" ${Math.min(30, m - 5)}m 95%`],
  "threshold.intervals": (m) => [`Threshold ${n((m + 4) / 12)}×8′`, `"Threshold" ${n((m + 4) / 12)}x[!last](8m 98%, 4m 50%)`],
  "threshold.hills": (m) => [`Threshold climbs ${n((m + 4) / 10)}×6′`, `"Climb" ${n((m + 4) / 10)}x[!last](6m 98% 70-80rpm, 4m 50%)`],
  "threshold.race": (m) => [`Break-away sim`, `"Race" ${n(m / 6)}x(5m 95%, 30s 100%, 30s 60%)`],
  "vo2max.intervals": (m) => [`30/15 VO2 sets ${n((m + 3) / 13)}×13`, `"VO2 sets" ${n((m + 3) / 13)}x[!last](13x(30s 120%, 15s 50%), 3m 50%)`],
  "vo2max.hills": (m) => [`VO2 climbs ${n((m + 4) / 8)}×4′`, `"Climb" ${n((m + 4) / 8)}x[!last](4m 115% 75-85rpm, 4m 50%)`],
  "vo2max.race": (m) => [`Race sim: attacks`, `"Race" ${n(m / 5)}x(4m 82%, 1m 120%)`],
  "sprint.intervals": (m) => [`Sprints ${n(m / 3)}×12″`, `"Sprints" ${n(m / 3)}x(12s 180% 110-120rpm, 2m48s 50%)`],
  "sprint.hills": (m) => [`Kickers ${n(m / 4)}×20″`, `"Kicker" ${n(m / 4)}x(20s 150% 70-90rpm, 3m40s 55%)`],
  "sprint.race": (m) => [`Crit finale`, `"Race" ${n((m - 2) / 4)}x(3m 85%, 30s 130%, 30s 60%)\n"Sprint" 15s 180%\n"Easy" 1m45s 50%`],
};
export const coherent = (c: Cell) => `${c.intensity}.${c.format}` in SESSIONS;
export const ALL_CELLS: Cell[] = INTENSITIES.flatMap((intensity) => FORMATS.map((format) => ({ intensity, format }))).filter(coherent);

/**
 * The health cap (DESIGN §5.8: threshold, no sprints) removes every cell above threshold, every format whose
 * trainer mode does not bound power (SIM, free), and race sims (their surges are above CP): under a cap only
 * steady and interval ERG rides remain, so actual power can't pass it (§15.2).
 */
export const lockedByCap = (c: Cell, capped: boolean) => capped && (rank(c.intensity) > rank("threshold") || FORMAT_MODE[c.format] !== "ERG" || c.format === "race");
const allowed = (c: Cell, capped: boolean) => coherent(c) && !lockedByCap(c, capped);

/** Under a cap every planned target is ≤ 97 % of it (CAP_RULE.planPct; the cap is 100 % of CP), so ERG ripple never reads as an exceedance. */
export function proposalFor(c: Cell, minutes: number, cp: number, capped = false): Proposal {
  const make = SESSIONS[`${c.intensity}.${c.format}`];
  if (!make) throw new Error(`no session for ${cellName(c)}`);
  const [name, raw] = make(minutes - 15);
  const main = capped ? raw.replace(/(\d+)%/g, (m, p: string) => (Number(p) > CAP_RULE.planPct ? `${CAP_RULE.planPct}%` : m)) : raw;
  const r = parseDsl(`"Warm-up" 10m ramp 45-70%\n${main}\n"Cool-down" 5m ramp 60-40%`, cp);
  if ("error" in r) throw new Error(r.error.message);
  return { workout: { id: `wish-${c.intensity}-${c.format}`, name, ftp: cp, nodes: r.nodes }, rationale: cellName(c) };
}

/** What crank actually measures or asks; a "crank uses" line may only name these (DESIGN §15.3, build check 5). */
export const INPUTS = { legs: "your legs answer", sleep: "your sleep answer", time: "the time you have", load7d: "your last 7 days of rides", cap: "your injury answer", profile: "your training profile", history: "your ride history" } as const;
export type InputId = keyof typeof INPUTS;
/** One reason: the facts crank saw, the inputs it used, the pack rule, and the grade of the cut-off it applied. */
export interface WishReason { kind: "readiness" | "load" | "safety" | "session" | "profile"; text: string; facts: string[]; uses: InputId[]; rule?: string; cutoff?: { grade: Grade; what: string } }
/** How the planner treated the wish (DESIGN §15.4 `WishOutcome`). */
export interface WishOutcome {
  wish: Wish;
  delivered: Cell;
  /** the time the proposal uses; less than asked only when a reason says so (adapted) */
  minutes: number;
  relation: "no_wish" | "as_wished" | "adapted" | "deferred" | "blocked_by_safety";
  reasons: WishReason[];
  deferredTo: string | null;
  /** readiness deviations only: never safety (DESIGN §15.2) */
  overridable: boolean;
}
export interface PlanContext { legs: number; sleep: number; minutes: number; capped: boolean; coachPick: Cell }

const TWO_SIGNALS = { grade: "D", what: "“two signals” as the cut-off for easing a hard session" } as const;
export const fmtMin = (m: number) => (m >= 120 ? `${m / 60} h` : m === 90 ? "1:30 h" : `${m} min`);

/** Planner stand-in: wish + check-in + cap → what is proposed and why. Safety is applied first and never yields. */
export function planWish(wish: Wish, ctx: PlanContext): WishOutcome {
  const base = { wish, minutes: ctx.minutes, deferredTo: null, overridable: false };
  if (!wish) return { ...base, delivered: ctx.coachPick, relation: "no_wish", reasons: [] };
  if (lockedByCap(wish, ctx.capped)) {
    const format: Format = wish.format === "open" ? "steady" : wish.format === "steady" ? "steady" : "intervals";
    const c: Cell = { intensity: rank(wish.intensity) > rank("threshold") ? "threshold" : wish.intensity, format };
    return { ...base, delivered: allowed(c, true) ? c : ctx.coachPick, relation: "blocked_by_safety", reasons: [{ kind: "safety", text: "Your injury cap: nothing above threshold, no sprints or race surges, trainer in ERG only.", facts: ["injury that limits hard efforts: yes"], uses: ["cap"] }] };
  }
  const legsWord = ctx.legs === 1 ? "dead" : "heavy";
  const sleepWord = ctx.sleep === 1 ? "poor" : "short";
  const against = [ctx.legs <= 2 && `legs: ${legsWord}`, ctx.sleep <= 2 && `sleep: ${sleepWord}`].filter((x): x is string => !!x);
  if (against.length >= 2 && rank(wish.intensity) >= rank("threshold")) {
    const easier: Cell = { intensity: "tempo", format: allowed({ intensity: "tempo", format: wish.format }, ctx.capped) ? wish.format : "intervals" };
    return {
      ...base,
      delivered: easier,
      relation: "deferred",
      reasons: [{ kind: "readiness", text: `Legs ${legsWord}, sleep ${sleepWord}: two signals against hard work today.`, facts: against, uses: ["legs", "sleep"], rule: "R-RDY-02", cutoff: TWO_SIGNALS }],
      deferredTo: "Thu 15 Oct",
      overridable: true,
    };
  }
  if (ctx.sleep <= 2 && ctx.minutes > 60 && (wish.format === "steady" || wish.format === "open"))
    return {
      ...base,
      delivered: wish,
      minutes: 60,
      relation: "adapted",
      reasons: [{ kind: "readiness", text: `Sleep ${sleepWord}: long continuous efforts are cut first — 60 min, not ${fmtMin(ctx.minutes)}.`, facts: [`sleep: ${sleepWord}`, `time: ${fmtMin(ctx.minutes)}`], uses: ["sleep", "time"], rule: "R-RDY-03", cutoff: { grade: "D", what: "60 min as the shortened length" } }],
      overridable: true,
    };
  const notes: WishReason[] = wish.intensity === "vo2max" && wish.format === "intervals" ? [{ kind: "session", text: "VO2max as 30/15 sets: the default VO2max session for trained riders.", facts: [], uses: ["history"], rule: "R-INT-01" }] : [];
  return { ...base, delivered: wish, relation: "as_wished", reasons: notes };
}

// ---------- saddle gestures (DESIGN §15.2): bounded steps, nothing selects a cell that is not a choice ----------
/** Intensity −/+ (and Easier / Harder): one row within the format, bounded, skipping cells that are not choices. */
export function stepWish(from: Cell, dir: -1 | 1, capped: boolean): Cell {
  for (let i = rank(from.intensity) + dir; i >= 0 && i < INTENSITIES.length; i += dir) {
    const c: Cell = { intensity: INTENSITIES[i] ?? "easy", format: from.format };
    if (allowed(c, capped)) return c;
  }
  return from;
}
/** Remote wish mode, Skip: the next format (wraps — a picker; nothing applies until Coach), nearest intensity. */
export function nextFormat(from: Cell, capped: boolean): Cell {
  for (let k = 1; k <= FORMATS.length; k++) {
    const format = FORMATS[(FORMATS.indexOf(from.format) + k) % FORMATS.length] ?? "steady";
    const best = INTENSITIES.map((intensity) => ({ intensity, format }))
      .filter((c) => allowed(c, capped))
      .sort((a, b) => Math.abs(rank(a.intensity) - rank(from.intensity)) - Math.abs(rank(b.intensity) - rank(from.intensity)))[0];
    if (best) return best;
  }
  return from;
}
/** Remote wish mode, Pause: the next choice in reading order (the two-button path reaches every cell). */
export function nextCell(from: Cell, capped: boolean): Cell {
  const cells = ALL_CELLS.filter((c) => allowed(c, capped));
  return cells[(cells.findIndex((c) => sameCell(c, from)) + 1) % cells.length] ?? from;
}

// ---------- a deferred wish carries over with a fixed expiry (DESIGN §15.2 lifecycle) ----------
export interface WishRecord { id: string; cell: Cell; setOn: string; forDate: string; expires: string; status: "open" | "deferred" | "fulfilled" | "replaced" | "overridden" | "expired" }
export const carriedFixture: WishRecord = { id: "w-0413", cell: { intensity: "vo2max", format: "intervals" }, setOn: "Sun 11 Oct", forDate: "Tue 13 Oct", expires: "Sun 18 Oct", status: "deferred" };

// ---------- training profile (#85) ----------
export const FOCI = [
  ["base", "Endurance base", "long, mostly easy riding"],
  ["climbing", "Climbing", "sustained efforts at and around threshold"],
  ["racing", "Racing (crits, road)", "surges, sprints, VO2max"],
  ["long_events", "Long events", "gran fondo, sportive: strong after hours"],
  ["tt", "Time trial", "steady power at threshold"],
  ["fitness", "General fitness & health", "fit and healthy, no event"],
] as const;
export type Focus = (typeof FOCI)[number][0];
export interface TrainingProfile {
  version: number;
  focus: Focus;
  event: { name: string; date: string } | null;
  daysPerWeek: number;
  hoursPerWeek: number;
  likes: Format[];
  returning: { since: string } | null;
}
export const profileFixture: TrainingProfile = { version: 3, focus: "long_events", event: { name: "Engadin Radmarathon", date: "2027-07-11" }, daysPerWeek: 4, hoursPerWeek: 7, likes: ["steady", "hills"], returning: null };
export const defaultProfile: TrainingProfile = { version: 1, focus: "fitness", event: null, daysPerWeek: 3, hoursPerWeek: 4, likes: [], returning: null };
/** "Coming back" uses the athlete model's gap threshold (spec/ONBOARDING §3): one number everywhere. */
export const RETURN_GAP_DAYS = 10;

/** One line of "what this changes": its evidence (rule) and the conditions it needs (`when`). */
export interface Effect { id: string; text: string; rule?: string; when?: string }
export function profileEffects(p: TrainingProfile): Effect[] {
  const out: Effect[] = [{ id: "tid", text: "Most of the week easy, 1–2 (at most 3) hard sessions — whatever the goal.", rule: "R-TID-01" }];
  if (p.returning) out.push({ id: "return", text: "A return block first: easier weeks, wider uncertainty; it ends by itself when your numbers settle.", when: `after ${RETURN_GAP_DAYS}+ days off` });
  if (p.focus === "long_events" || p.focus === "base") {
    const can = p.hoursPerWeek >= 5 && !p.returning;
    out.push({
      id: "durability",
      text: can ? "Optional: now and then a quality effort after 2 h or more, fuelled — suggested, never scheduled on its own." : "Not now: a quality effort after 2 h or more.",
      rule: "R-DUR-01",
      when: can ? `on days with 2 h+ to ride (budget ${p.hoursPerWeek} h/week)` : p.returning ? "not during a return block" : `needs 5 h/week or more (yours: ${p.hoursPerWeek} h)`,
    });
    out.push({ id: "cho", text: "Fuelling guideline (training, not diet): rides over 75–90 min come with a carbohydrate target.", rule: "R-CHO-01" });
  }
  if (p.focus === "racing" || p.focus === "climbing") out.push({ id: "vo2", text: "VO2max work as 30/15 sets, alternating with 4–5′ intervals.", rule: "R-INT-01" });
  out.push({ id: `mix-${p.focus}`, text: `Which hard sessions suit “${FOCI.find((f) => f[0] === p.focus)?.[1]}” is coaching convention: no trial in the pack compares goal-specific mixes.` });
  if (p.event) out.push({ id: "taper", text: "No peaks or tapers planned from fitness/fatigue curves (CTL/TSB) alone.", rule: "R-LOAD-01" });
  return out;
}
