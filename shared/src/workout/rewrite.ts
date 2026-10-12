import type { RepeatNode, RewriteIntent, StepNode, WorkoutNode } from "../contract";
import type { Rng } from "../rng";
import { uid } from "../util";
import { isWorkStep, shiftTarget, targetPct } from "./model";

/*
 * Deterministic (given the rng) tree transforms behind planner.rewrite. Node ids are kept for
 * nodes that were modified in place; nodes that are newly created get fresh ids, so a UI can
 * diff old vs new by id. Inputs are never mutated.
 */

const r5 = (s: number) => Math.max(5, Math.round(s / 5) * 5);

function containsWork(n: WorkoutNode, ftp: number): boolean {
  return n.type === "step" ? isWorkStep(n, ftp) : n.children.some((c) => containsWork(c, ftp));
}

function mapSteps(nodes: readonly WorkoutNode[], f: (s: StepNode) => WorkoutNode): WorkoutNode[] {
  return nodes.map((n) => (n.type === "step" ? f(n) : { ...n, children: mapSteps(n.children, f) }));
}

function scaleTime(s: StepNode, k: number, min = 5): StepNode {
  if (s.end.kind !== "time") return s;
  return { ...s, end: { kind: "time", s: Math.max(min, r5(s.end.s * k)) } };
}

/** Apply `f` to the outermost repeats that contain work; other nodes go through `g`. */
function mapOuterWorkRepeats(nodes: readonly WorkoutNode[], ftp: number, f: (r: RepeatNode) => RepeatNode): WorkoutNode[] {
  return nodes.map((n) => {
    if (n.type === "repeat") return containsWork(n, ftp) ? f(n) : n;
    return n;
  });
}

function harder(nodes: readonly WorkoutNode[], ftp: number): WorkoutNode[] {
  const hasRepeat = nodes.some((n) => n.type === "repeat" && containsWork(n, ftp));
  if (hasRepeat) {
    const more = mapOuterWorkRepeats(nodes, ftp, (r) => ({ ...r, count: Math.min(20, r.count + 1) }));
    return mapSteps(more, (s) => (isWorkStep(s, ftp) ? { ...s, target: shiftTarget(s.target, 2, ftp) } : s));
  }
  return mapSteps(nodes, (s) => (isWorkStep(s, ftp) ? scaleTime({ ...s, target: shiftTarget(s.target, 5, ftp) }, 1.1) : s));
}

function easier(nodes: readonly WorkoutNode[], ftp: number): WorkoutNode[] {
  const fewer = mapOuterWorkRepeats(nodes, ftp, (r) => ({ ...r, count: Math.max(1, r.count - 1) }));
  return mapSteps(fewer, (s) => {
    if (isWorkStep(s, ftp)) return { ...s, target: shiftTarget(s.target, -5, ftp) };
    if (s.end.kind === "hr_recovery") return { ...s, end: { ...s.end, targetBpm: s.end.targetBpm + 5 } };
    return scaleTime(s, 1.25);
  });
}

function shorterSameStimulus(nodes: readonly WorkoutNode[], ftp: number): WorkoutNode[] {
  return mapSteps(nodes, (s) => {
    if (isWorkStep(s, ftp)) return scaleTime({ ...s, target: shiftTarget(s.target, 4, ftp) }, 0.8);
    if (s.end.kind === "hr_recovery") return { ...s, end: { ...s.end, minS: r5(s.end.minS * 0.7), maxS: r5(s.end.maxS * 0.8) } };
    if (s.end.kind === "time" && s.end.s >= 300) return scaleTime(s, 0.75, 180); // warm-up / cool-down / long easy
    return scaleTime(s, 0.7, 10); // rests
  });
}

function vary(nodes: readonly WorkoutNode[], ftp: number, rng: Rng): WorkoutNode[] {
  let changed = false;
  const visit = (ns: readonly WorkoutNode[]): WorkoutNode[] =>
    ns.flatMap((n): WorkoutNode[] => {
      if (n.type === "repeat") {
        const work = n.children.filter((c): c is StepNode => c.type === "step" && isWorkStep(c, ftp));
        const short = work.length > 0 && work.every((c) => c.end.kind === "time" && c.end.s < 60);
        if (short && rng.chance(0.75)) {
          changed = true;
          const longer = rng.chance(0.5);
          const k = longer ? 1.5 : 2 / 3;
          const dp = longer ? -6 : 6;
          return [
            {
              ...n,
              count: Math.max(2, Math.round(n.count / k)),
              children: n.children.map((c) => {
                if (c.type !== "step") return c;
                const s = scaleTime(c, k);
                return isWorkStep(c, ftp) ? { ...s, target: shiftTarget(s.target, dp, ftp) } : s;
              }),
            },
          ];
        }
        return [{ ...n, children: visit(n.children) }];
      }
      if (!isWorkStep(n, ftp) || n.end.kind !== "time" || n.end.s < 120) return [n];
      const v = targetPct(n.target, ftp);
      const d = n.end.s;
      const choice = rng.int(0, 2);
      changed = true;
      if (choice === 0) {
        // over-under split as a nested repeat
        const half = r5(d / 4);
        const rep: RepeatNode = {
          type: "repeat",
          id: uid("r"),
          label: n.label,
          count: 2,
          children: [
            { type: "step", id: n.id, end: { kind: "time", s: half }, target: { kind: "pct_ftp", value: Math.round(v - 5) } },
            { type: "step", id: uid("s"), end: { kind: "time", s: half }, target: { kind: "pct_ftp", value: Math.round(v + 5) } },
          ],
        };
        if (rep.label === undefined) delete rep.label;
        return [rep];
      }
      if (choice === 1) return [{ ...n, target: { kind: "ramp", from: Math.round(v - 7), to: Math.round(v + 7) } }];
      // pyramid: three thirds
      const third = r5(d / 3);
      return [
        { ...n, end: { kind: "time", s: third }, target: { kind: "pct_ftp", value: Math.round(v - 4) } },
        { type: "step", id: uid("s"), end: { kind: "time", s: third }, target: { kind: "pct_ftp", value: Math.round(v + 6) } },
        { type: "step", id: uid("s"), end: { kind: "time", s: third }, target: { kind: "pct_ftp", value: Math.round(v - 2) } },
      ];
    });
  const out = visit(nodes);
  if (changed) return out;
  // nothing structural to vary: nudge work intensities a little
  const dp = rng.chance(0.5) ? 3 : -3;
  return mapSteps(out, (s) => (isWorkStep(s, ftp) ? { ...s, target: shiftTarget(s.target, dp, ftp) } : scaleTime(s, 0.9)));
}

export function rewriteNodes(nodes: readonly WorkoutNode[], intent: RewriteIntent, ftp: number, rng: Rng): WorkoutNode[] {
  const src = structuredClone(nodes) as WorkoutNode[];
  switch (intent) {
    case "harder":
      return harder(src, ftp);
    case "easier":
      return easier(src, ftp);
    case "shorter_same_stimulus":
      return shorterSameStimulus(src, ftp);
    case "vary":
      return vary(src, ftp, rng);
  }
}
