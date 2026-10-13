// Contract stand-ins for the training profile (#85) and the planner wish (#86): DESIGN §15. The planner
// decides how a wish is treated and says why (contract fields for #59, DESIGN §15.4); this module fakes
// that decision for the prototype, like ext.ts. The production UI must not import it.
import { parseDsl, type Proposal } from "@crank/shared";

/** Intensity axis, low → high, anchored on the athlete's thresholds (R-GEN-01), not on zone numbers. */
export const INTENSITIES = ["easy", "tempo", "threshold", "vo2max", "sprint"] as const;
export const FORMATS = ["intervals", "hills", "long", "race", "open"] as const;
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
export const FORMAT_NAME: Record<Format, string> = { intervals: "Intervals", hills: "Hills", long: "Long", race: "Race sim", open: "Open ride" };
export const cellName = (c: Cell) => `${INTENSITY_NAME[c.intensity][0]} · ${FORMAT_NAME[c.format]}`;
const rank = (i: Intensity) => INTENSITIES.indexOf(i);

/**
 * The sessions behind each coherent cell (name, main set as DSL with % of CP). Cells not listed are not
 * choices: "VO2max · Long" or "Easy · Race sim" describe no session (DESIGN §15.2).
 */
const SESSIONS: Partial<Record<`${Intensity}.${Format}`, (main: number) => [string, string]>> = {
  "easy.hills": (m) => [`Rolling easy`, `"Rolling" ${Math.max(1, Math.floor(m / 6))}x(4m 60%, 2m 72% 75-85rpm)`],
  "easy.long": (m) => [`Endurance`, `"Endurance" ${m}m 65%`],
  "easy.open": (m) => [`Open ride, easy ceiling`, `"Open" ${m}m free`],
  "tempo.intervals": (m) => [`Tempo ${Math.max(1, Math.floor((m + 5) / 15))}×10′`, `"Tempo" ${Math.max(1, Math.floor((m + 5) / 15))}x[!last](10m 85% 85-95rpm, 5m 55%)`],
  "tempo.hills": (m) => [`Tempo climbs ${Math.max(1, Math.floor((m + 4) / 12))}×8′`, `"Climb" ${Math.max(1, Math.floor((m + 4) / 12))}x[!last](8m 85% 70-80rpm, 4m 55%)`],
  "tempo.long": (m) => [`Long with tempo`, `"Endurance" ${Math.max(5, m - 20)}m 65%\n"Tempo" 20m 82%`],
  "tempo.open": (m) => [`Open ride, tempo ceiling`, `"Open" ${m}m free`],
  "threshold.intervals": (m) => [`Threshold ${Math.max(1, Math.floor((m + 4) / 12))}×8′`, `"Threshold" ${Math.max(1, Math.floor((m + 4) / 12))}x[!last](8m 98%, 4m 50%)`],
  "threshold.hills": (m) => [`Threshold climbs ${Math.max(1, Math.floor((m + 4) / 10))}×6′`, `"Climb" ${Math.max(1, Math.floor((m + 4) / 10))}x[!last](6m 98% 70-80rpm, 4m 50%)`],
  "threshold.long": (m) => [`Long, threshold late`, `"Endurance" ${Math.max(5, m - 24)}m 65%\n"Threshold" 2x(8m 95%, 4m 55%)`],
  "threshold.race": (m) => [`TT effort ${Math.min(20, m - 5)}′`, `"TT" ${Math.min(20, m - 5)}m 95%\n"Easy" 5m 50%`],
  "vo2max.intervals": (m) => [`30/15 VO2 sets ${Math.max(1, Math.floor((m + 3) / 13))}×13`, `"VO2 sets" ${Math.max(1, Math.floor((m + 3) / 13))}x[!last](13x(30s 120%, 15s 50%), 3m 50%)`],
  "vo2max.hills": (m) => [`VO2 climbs ${Math.max(1, Math.floor((m + 4) / 8))}×4′`, `"Climb" ${Math.max(1, Math.floor((m + 4) / 8))}x[!last](4m 115% 75-85rpm, 4m 50%)`],
  "vo2max.race": (m) => [`Race sim: surges`, `"Race" ${Math.max(1, Math.floor(m / 5))}x(4m 82%, 1m 120%)`],
  "sprint.intervals": (m) => [`Sprints ${Math.max(1, Math.floor(m / 3))}×12″`, `"Sprints" ${Math.max(1, Math.floor(m / 3))}x(12s 180% 110-120rpm, 2m48s 50%)`],
  "sprint.hills": (m) => [`Kickers ${Math.max(1, Math.floor(m / 4))}×20″`, `"Kicker" ${Math.max(1, Math.floor(m / 4))}x(20s 150% 70-90rpm, 3m40s 55%)`],
  "sprint.race": (m) => [`Crit finale`, `"Race" ${Math.max(1, Math.floor((m - 2) / 4))}x(3m 85%, 30s 130%, 30s 60%)\n"Sprint" 15s 180%\n"Easy" 1m45s 50%`],
};
export const coherent = (c: Cell) => `${c.intensity}.${c.format}` in SESSIONS;

/** The health cap (DESIGN §5.8: threshold, no sprints) removes every cell above threshold from the choices. */
export const lockedByCap = (c: Cell, capped: boolean) => capped && rank(c.intensity) > rank("threshold");

/** Long rides need time; the planner shortens them to the day's budget and says so. */
const LONG_MIN = 90;

export function proposalFor(c: Cell, minutes: number, cp: number): Proposal {
  const make = SESSIONS[`${c.intensity}.${c.format}`];
  if (!make) throw new Error(`no session for ${cellName(c)}`);
  const [name, main] = make(minutes - 15);
  const r = parseDsl(`"Warm-up" 10m ramp 45-70%\n${main}\n"Cool-down" 5m ramp 60-40%`, cp);
  if ("error" in r) throw new Error(r.error.message);
  return { workout: { id: `wish-${c.intensity}-${c.format}`, name, ftp: cp, nodes: r.nodes }, rationale: cellName(c) };
}

/** One reason for a deviation or a note on a granted wish: facts first, then the pack rule it rests on. */
export interface WishReason { kind: "readiness" | "load" | "time" | "safety" | "session"; text: string; rule?: string }
/** How the planner treated the wish (DESIGN §15.4 `WishOutcome`). */
export interface WishOutcome {
  wish: Wish;
  delivered: Cell;
  relation: "no_wish" | "as_wished" | "adapted" | "deferred" | "blocked_by_safety";
  reasons: WishReason[];
  deferredTo: string | null;
  /** only readiness deviations can be overridden; never safety, and time is the rider's own answer (DESIGN §15.2) */
  overridable: boolean;
}

export interface PlanContext { legs: number; sleep: number; minutes: number; capped: boolean; coachPick: Cell }

/** Planner stand-in: wish + check-in + cap → what is proposed and why. Safety is applied first and never yields. */
export function planWish(wish: Wish, ctx: PlanContext): WishOutcome {
  if (!wish) return { wish, delivered: ctx.coachPick, relation: "no_wish", reasons: [], deferredTo: null, overridable: false };
  if (lockedByCap(wish, ctx.capped))
    return {
      wish,
      delivered: { intensity: "tempo", format: coherent({ intensity: "tempo", format: wish.format }) ? wish.format : "intervals" },
      relation: "blocked_by_safety",
      reasons: [{ kind: "safety", text: "Capped at threshold by your injury answer: no VO2max or sprints until you lift it in Settings → Health." }],
      deferredTo: null,
      overridable: false,
    };
  const against = [ctx.legs <= 2 && `legs ${ctx.legs === 1 ? "dead" : "heavy"}`, ctx.sleep <= 2 && `sleep ${ctx.sleep === 1 ? "poor" : "short"}`].filter(Boolean);
  if (against.length >= 2 && rank(wish.intensity) >= rank("threshold")) {
    const easier: Cell = { intensity: "tempo", format: coherent({ intensity: "tempo", format: wish.format }) ? wish.format : "intervals" };
    return {
      wish,
      delivered: easier,
      relation: "deferred",
      reasons: [{ kind: "readiness", text: `You said ${against.join(" and ")}: two signals against hard work today.`, rule: "R-RDY-02" }],
      deferredTo: "Thu 15 Oct, if your check-in is ok",
      overridable: true,
    };
  }
  if (wish.format === "long" && ctx.minutes < LONG_MIN)
    return { wish, delivered: wish, relation: "adapted", reasons: [{ kind: "time", text: `A long ride needs ${LONG_MIN} min or more; you have ${ctx.minutes} min, so it is ridden as ${ctx.minutes}′.` }], deferredTo: null, overridable: false };
  const notes: WishReason[] = wish.intensity === "vo2max" && wish.format === "intervals" ? [{ kind: "session", text: "VO2max as 30/15 sets: the default VO2max session for trained riders.", rule: "R-INT-01" }] : [];
  return { wish, delivered: wish, relation: "as_wished", reasons: notes, deferredTo: null, overridable: false };
}

/** Saddle gesture (DESIGN §15.2): Intensity −/+ move the wish along the intensity axis, bounded, skipping cells that are not choices. */
export function stepWish(from: Cell, dir: -1 | 1, capped: boolean): Cell {
  for (let i = rank(from.intensity) + dir; i >= 0 && i < INTENSITIES.length; i += dir) {
    const c: Cell = { intensity: INTENSITIES[i] ?? "easy", format: from.format };
    if (coherent(c) && !lockedByCap(c, capped)) return c;
  }
  return from;
}

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
  focus: Focus;
  event: { name: string; date: string } | null;
  daysPerWeek: number;
  hoursPerWeek: number;
  likes: Format[];
  returning: { since: string } | null;
}
export const profileFixture: TrainingProfile = { focus: "long_events", event: { name: "Engadin Radmarathon", date: "2027-07-11" }, daysPerWeek: 4, hoursPerWeek: 7, likes: ["long", "hills"], returning: null };

/** What the profile changes in the planner's prior, each line with its evidence (DESIGN §15.1). */
export function profileEffects(p: TrainingProfile): WishReason[] {
  const out: WishReason[] = [{ kind: "session", text: "Most of the week easy, 1–2 (at most 3) hard sessions — whatever the goal.", rule: "R-TID-01" }];
  if (p.focus === "long_events" || p.focus === "base") out.push({ kind: "session", text: "One session a week with a quality effort after 2 h or more, fuelled.", rule: "R-DUR-01" }, { kind: "session", text: "Every ride over 75–90 min comes with a carbohydrate target.", rule: "R-CHO-01" });
  if (p.focus === "racing" || p.focus === "climbing") out.push({ kind: "session", text: "VO2max work as 30/15 sets, alternating with 4–5′ intervals.", rule: "R-INT-01" });
  out.push({ kind: "load", text: `Which hard sessions suit “${FOCI.find((f) => f[0] === p.focus)?.[1]}” is coaching convention: no trial in the pack compares goal-specific mixes.` });
  if (p.event) out.push({ kind: "load", text: "Peaks and tapers are not planned from fitness/fatigue curves (CTL/TSB) alone.", rule: "R-LOAD-01" });
  return out;
}
