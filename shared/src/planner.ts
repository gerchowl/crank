import type { Checkin, Proposal } from "./contract";
import { hashString } from "./rng";
import { compileWorkout } from "./workout/compile";
import { dsl } from "./workout/library";

/** Mock planner latency (ms): ~800 ms, varying deterministically with the check-in. */
export function plannerLatencyMs(c: Checkin): number {
  const h = hashString(JSON.stringify(c));
  return 600 + (h % 401); // 600–1000 ms
}

const NEGATIVE = /(tired|sick|ill|sore|exhausted|bad|cold|knackered|heavy)/i;
const POSITIVE = /(great|fresh|strong|good|fit|motivated|ready)/i;

/** Readiness 1..5 from the check-in (+ free-text hints). */
export function readiness(c: Checkin): number {
  let r = (c.sleep + c.legs + (6 - c.stress) + c.motivation) / 4;
  if (c.note && NEGATIVE.test(c.note)) r -= 0.75;
  if (c.note && POSITIVE.test(c.note)) r += 0.25;
  return Math.max(1, Math.min(5, r));
}

/**
 * Mock pre-ride planner: picks a session type from readiness and fills the available time.
 * The result is built from DSL so it round-trips through the editor.
 */
export function proposeWorkout(c: Checkin, ftp: number): Proposal {
  const r = readiness(c);
  const minutes = Math.max(20, Math.min(240, Math.round(c.minutes)));
  const wu = minutes >= 45 ? 10 : 7;
  const cd = minutes >= 45 ? 7 : 5;
  const main = Math.max(6, minutes - wu - cd);
  const head = `"Warm-up" ${wu}m ramp 45-75% 90rpm`;
  const tail = `"Cool-down" ${cd}m ramp 60-40%`;
  let body: string;
  let name: string;
  let why: string;
  const fmt = (x: number) => x.toFixed(1);

  if (r >= 3.8) {
    if (c.motivation >= 4 && (c.legs + c.sleep) % 2 === 0) {
      // 30/15s sets: each set 13×45 s + HR-guided recovery ≈ 12.5 min
      const sets = Math.max(1, Math.min(4, Math.floor(main / 12.5)));
      body = `"30/15s" ${sets}x[+5w](13x(30s 120%, 15s 50%), "Recover" hr<120 2m..4m 40%)`;
      name = `30/15s × ${sets}`;
      why = `Readiness ${fmt(r)}/5 — legs ${c.legs}/5, sleep ${c.sleep}/5: fresh enough for VO2max work. ${sets} sets of 30/15s with HR-guided recoveries (end when HR < 120).`;
    } else {
      const reps = Math.max(2, Math.min(6, Math.floor((main + 3) / 7)));
      body = `"4×4" ${reps}x[!last](4m 110% 95rpm, 3m 50%)`;
      name = `${reps}×4′ VO2max`;
      why = `Readiness ${fmt(r)}/5: good day for VO2max. ${reps}×4′ at 110 % with 3′ recoveries.`;
    }
  } else if (r >= 2.8) {
    if (c.stress >= 4) {
      const reps = Math.max(1, Math.min(4, Math.floor((main + 4) / 13)));
      body = `"Sweet spot" ${reps}x[!last](10m 90%, 3m 55%)`;
      name = `Sweet spot ${reps}×10′`;
      why = `Readiness ${fmt(r)}/5 with high stress (${c.stress}/5): steady sweet spot instead of high-intensity — controllable, still productive.`;
    } else {
      const sets = Math.max(1, Math.min(4, Math.floor((main + 4) / 13)));
      body = `"Over-unders" ${sets}x[!last](3x(2m 95%, 1m 108%), 4m 50%)`;
      name = `Over-unders ${sets}×9′`;
      why = `Readiness ${fmt(r)}/5: threshold work. ${sets} sets of 3×(2′ @95 % / 1′ @108 %).`;
    }
  } else if (r >= 2) {
    const blocks = Math.max(1, Math.floor(main / 15));
    const left = main - blocks * 15;
    body = `"Endurance" ${blocks}x(12m 65% 85-95rpm, 3m 70% 100-105rpm)${left >= 2 ? `\n${left}m 62%` : ""}`;
    name = `Endurance ${minutes}′`;
    why = `Readiness ${fmt(r)}/5: aerobic endurance only, with high-cadence drills. Save the intensity for a fresher day.`;
  } else {
    body = `"Recovery spin" ${main}m 50% 90rpm`;
    name = `Recovery spin ${minutes}′`;
    why = `Readiness ${fmt(r)}/5${c.note ? ` and you wrote “${c.note.slice(0, 60)}”` : ""}: easy recovery spin, HR in zone 1–2. Consider a rest day.`;
  }

  const nodes = dsl(`${head}\n${body}\n${tail}`, ftp);
  const workout = { id: `plan-${hashString(JSON.stringify(c)).toString(36)}`, name, ftp, nodes };
  const cw = compileWorkout(workout);
  const mins = Math.round(cw.totalS / 60);
  return { workout, rationale: `${why} ≈ ${mins}′, TSS ≈ ${Math.round(cw.tss)}, IF ${cw.if.toFixed(2)}.` };
}
