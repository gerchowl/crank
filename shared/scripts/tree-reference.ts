// ADR-0001 §3 / #152 / #159: reference results of the frozen tree helpers (src/workout/model.ts),
// consumed by crates/crank-domain/tests/tree_helpers.rs. Executes the mock, never reimplements it.
import type { WorkoutNode } from "../src/contract";
import { findNode, reId } from "../src/workout/model";

const step = (id: string, label?: string): WorkoutNode => ({
  type: "step",
  id,
  ...(label === undefined ? {} : { label }),
  end: { kind: "time", s: 60 },
  target: { kind: "pct_ftp", value: 90 },
  cadence: { lo: 85, hi: 95 },
});
const rep = (id: string, children: WorkoutNode[], extra: object = {}): WorkoutNode => ({
  type: "repeat",
  id,
  count: 3,
  children,
  ...extra,
});

/** The trees every case refers to by name, so each is stored once. */
export const trees: Record<string, WorkoutNode[]> = {
  empty: [],
  flat: [step("a", "warm"), step("b")],
  nested: [
    step("a"),
    rep("r1", [step("b"), rep("r2", [step("c", "deep"), step("d")], { skipLastRest: true })], {
      label: "main",
      progression: { wattsPerRep: 5, pctPerRep: 1.5 },
    }),
    step("e"),
  ],
  duplicate_ids: [rep("x", [step("y"), step("x")]), step("y"), rep("x", [step("z")])],
  empty_repeat: [rep("r", [])],
};

// Ids carry a random per-realm prefix; keep only the kind prefix ("s"/"r") so output is stable.
const stable = (nodes: WorkoutNode[]): WorkoutNode[] =>
  nodes.map((n) =>
    n.type === "step"
      ? { ...n, id: n.id.split("_")[0] ?? n.id }
      : { ...n, id: n.id.split("_")[0] ?? n.id, children: stable(n.children) },
  );
const preorder = (nodes: readonly WorkoutNode[]): string[] =>
  nodes.flatMap((n) => [n.id, ...(n.type === "repeat" ? preorder(n.children) : [])]);

/** Serialized fixture: `trees` once by name; `reId`/`findNode` cases refer to a tree by name. */
export function treeReference(): string {
  const reid = Object.entries(trees).map(([tree, input]) => {
    const out = reId(input);
    const ids = preorder(out);
    const old = new Set(preorder(input));
    return {
      tree,
      output: stable(out),
      freshUnique: new Set(ids).size === ids.length && ids.every((id) => !old.has(id)),
    };
  });
  const find = Object.entries(trees).flatMap(([tree, input]) =>
    [...new Set([...preorder(input), "missing", ""])].map((id) => ({
      tree,
      id,
      output: findNode(input, id) ?? null,
    })),
  );
  return JSON.stringify({ schemaVersion: 1, trees, reId: reid, findNode: find }, null, 1) + "\n";
}
