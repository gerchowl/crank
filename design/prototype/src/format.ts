// Presentation helpers (formatting and tree walking only — no physiology, DESIGN §10 #2).
import type { FlatBlock, RepeatNode, WorkoutNode } from "@crank/shared";

export const mmss = (s: number | null | undefined) => {
  if (s === null || s === undefined || !Number.isFinite(s)) return "–:––";
  const t = Math.max(0, Math.round(s));
  const h = Math.floor(t / 3600);
  const m = Math.floor((t % 3600) / 60);
  const ss = String(t % 60).padStart(2, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
};
export const num = (v: number | null | undefined, digits = 0) => (v === null || v === undefined || !Number.isFinite(v) ? "—" : v.toFixed(digits));
export const signed = (v: number) => `${v > 0 ? "+" : v < 0 ? "−" : "±"}${Math.abs(Math.round(v))}`;

/** Nodes along a FlatBlock path, outermost first. */
export function pathNodes(nodes: WorkoutNode[], path: number[]): WorkoutNode[] {
  const out: WorkoutNode[] = [];
  let level: WorkoutNode[] = nodes;
  for (const i of path) {
    const n = level[i];
    if (!n) break;
    out.push(n);
    level = n.type === "repeat" ? n.children : [];
  }
  return out;
}

const PHASE_NAME: Record<FlatBlock["phase"], string> = { warmup: "Warm-up", work: "Work", recovery: "Easy", cooldown: "Cool-down", free: "Free ride" };

/** What to call a block in "next": its own label, else its phase (work blocks: the enclosing label). */
export function blockName(nodes: WorkoutNode[], b: FlatBlock) {
  const along = pathNodes(nodes, b.path);
  const own = along.at(-1)?.label;
  if (own) return own;
  if (b.phase === "work") return [...along].reverse().find((n) => n.label)?.label ?? PHASE_NAME.work;
  return PHASE_NAME[b.phase];
}

/** Title + rep counters for a block: innermost label, "rep i/n" for every enclosing repeat. */
export function blockTitle(nodes: WorkoutNode[], b: FlatBlock) {
  const along = pathNodes(nodes, b.path);
  const label = [...along].reverse().find((n) => n.label)?.label;
  const repeats = along.filter((n): n is RepeatNode => n.type === "repeat");
  const reps = repeats.map((r, i) => ({ i: (b.iteration[i] ?? 0) + 1, n: r.count }));
  return { label: label ?? "", reps };
}
