import type { Workout, WorkoutNode } from "../contract";
import { parseDsl } from "./dsl";

/** Parse DSL that is known to be valid (library content); throws with context otherwise. */
export function dsl(text: string, ftp = 280): WorkoutNode[] {
  const r = parseDsl(text, ftp);
  if ("error" in r) throw new Error(`DSL error at ${r.error.offset}: ${r.error.message}\n${text}`);
  return r.nodes;
}

export const SNIPPET_DSL: readonly { name: string; text: string }[] = [
  { name: "30/15s", text: `"30/15s" 3x[!last](13x(30s 120%, 15s 50%), 3m 50%)` },
  { name: "4×4", text: `"4×4" 4x[!last](4m 110%, 3m 50%)` },
  { name: "Over-unders", text: `"Over-unders" 3x[!last](3x(2m 95%, 1m 108%), 4m 50%)` },
  { name: "Tabata", text: `"Tabata" 8x(20s 170%, 10s 30%)` },
];

/** Seed snippet library (fresh ids on every call). */
export function seedSnippets(): { name: string; nodes: WorkoutNode[] }[] {
  return SNIPPET_DSL.map((s) => ({ name: s.name, nodes: dsl(s.text) }));
}

/**
 * Default ~45-min demo: warm-up ramp, nested 30/15 sets with progression and HR-guided
 * recoveries between sets, threshold pair, cool-down.
 */
export const DEMO_DSL = `"Warm-up" 8m ramp 45-75% 90rpm
3m 60%
"VO2 sets" 3x[+5w](5x(30s 125%, 15s 50%), "Recover" hr<118 2m..4m 40%)
"Threshold" 2x[!last](4m 95% 90-95rpm, 2m 55%)
"Cool-down" 5m ramp 60-40%`;

export function demoWorkout(ftp = 280): Workout {
  return { id: "demo-45", name: "Demo: VO2 sets + threshold (45′)", ftp, nodes: dsl(DEMO_DSL, ftp) };
}
