import type { PowerTarget, StepNode, WorkoutNode } from "../contract";
import { type Athlete, DEFAULT_ATHLETE, powerForHr } from "../athlete";
import { uid } from "../util";

/** Steps at or above this intensity (% FTP) count as "work" (phase + progression). */
export const WORK_THRESHOLD_PCT = 80;

/** Representative intensity of a target in % FTP (ramp: max end, range: midpoint). */
export function targetPct(t: PowerTarget, ftp: number, athlete: Athlete = DEFAULT_ATHLETE): number {
  switch (t.kind) {
    case "pct_ftp":
      return t.value;
    case "watts":
      return (t.value / ftp) * 100;
    case "range":
      return (t.lo + t.hi) / 2;
    case "ramp":
      return Math.max(t.from, t.to);
    case "hr":
      return (powerForHr(t.bpm, { ...athlete, ftp }) / ftp) * 100;
    case "free":
      return 50;
  }
}

export interface Offset { watts: number; pct: number }

/** Start/end watts of a target, with a progression offset applied. */
export function targetWatts(t: PowerTarget, ftp: number, off: Offset = { watts: 0, pct: 0 }, athlete: Athlete = DEFAULT_ATHLETE): [number, number] {
  const w = (pct: number) => Math.max(0, ((pct + off.pct) / 100) * ftp + off.watts);
  switch (t.kind) {
    case "pct_ftp":
      return [w(t.value), w(t.value)];
    case "watts": {
      const v = Math.max(0, t.value + off.watts + (off.pct / 100) * ftp);
      return [v, v];
    }
    case "range": {
      const m = w((t.lo + t.hi) / 2);
      return [m, m];
    }
    case "ramp":
      return [w(t.from), w(t.to)];
    case "hr": {
      const p = powerForHr(t.bpm, { ...athlete, ftp });
      return [p, p];
    }
    case "free":
      return [0.5 * ftp, 0.5 * ftp];
  }
}

export function isWorkStep(s: StepNode, ftp: number): boolean {
  return s.end.kind !== "hr_recovery" && s.target.kind !== "free" && s.target.kind !== "hr" && targetPct(s.target, ftp) >= WORK_THRESHOLD_PCT;
}

/** Scale/shift a target by a delta in % FTP (watts targets are converted proportionally). */
export function shiftTarget(t: PowerTarget, deltaPct: number, ftp: number): PowerTarget {
  const r = (x: number) => Math.round(x * 10) / 10;
  switch (t.kind) {
    case "pct_ftp":
      return { kind: "pct_ftp", value: r(Math.max(0, t.value + deltaPct)) };
    case "watts":
      return { kind: "watts", value: Math.round(Math.max(0, t.value + (deltaPct / 100) * ftp)) };
    case "range":
      return { kind: "range", lo: r(Math.max(0, t.lo + deltaPct)), hi: r(Math.max(0, t.hi + deltaPct)) };
    case "ramp":
      return { kind: "ramp", from: r(Math.max(0, t.from + deltaPct)), to: r(Math.max(0, t.to + deltaPct)) };
    case "hr":
    case "free":
      return t;
  }
}

/** Deep copy with fresh ids on every node (use when inserting snippets/duplicating). */
export function reId(nodes: readonly WorkoutNode[]): WorkoutNode[] {
  return nodes.map((n) =>
    n.type === "step" ? { ...structuredClone(n), id: uid("s") } : { ...structuredClone(n), id: uid("r"), children: reId(n.children) },
  );
}

/** Find a node by id (depth-first). */
export function findNode(nodes: readonly WorkoutNode[], id: string): WorkoutNode | undefined {
  for (const n of nodes) {
    if (n.id === id) return n;
    if (n.type === "repeat") {
      const f = findNode(n.children, id);
      if (f) return f;
    }
  }
  return undefined;
}

/** Strip ids (for structural comparison in tests / diffs). */
export function withoutIds(nodes: readonly WorkoutNode[]): unknown[] {
  return nodes.map((n) => {
    if (n.type === "step") {
      const { id: _id, ...rest } = n;
      return rest;
    }
    const { id: _id, children, ...rest } = n;
    return { ...rest, children: withoutIds(children) };
  });
}
