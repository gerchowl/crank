// Regenerates tree_helpers.json from the frozen TS reference (shared/src/workout/model.ts).
// Run from the repository root:
//   pnpm -C shared exec vitest run --root .. crates/crank-domain/tests/fixtures/export-tree-reference.test.ts
// The fixture is committed; rerun only if the frozen reference changes. shared/ is not modified.
import { writeFileSync } from "node:fs";
import { test } from "vitest";
import type { WorkoutNode } from "../../../../shared/src/contract";
import { findNode, reId } from "../../../../shared/src/workout/model";

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

const trees: Record<string, WorkoutNode[]> = {
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
      ? { ...n, id: n.id.split("_")[0] }
      : { ...n, id: n.id.split("_")[0], children: stable(n.children) },
  );
const preorder = (nodes: readonly WorkoutNode[]): string[] =>
  nodes.flatMap((n) => [n.id, ...(n.type === "repeat" ? preorder(n.children) : [])]);

test("export tree helper reference", () => {
  const reid = Object.entries(trees).map(([name, input]) => {
    const out = reId(input);
    const ids = preorder(out);
    return {
      name,
      input,
      output: stable(out),
      freshUnique: new Set(ids).size === ids.length && ids.every((i) => !preorder(input).includes(i)),
    };
  });
  const find = Object.entries(trees).flatMap(([name, input]) =>
    [...new Set([...preorder(input), "missing", ""])].map((id) => ({
      name: `${name}/${id}`,
      input,
      id,
      output: findNode(input, id) ?? null,
    })),
  );
  writeFileSync(
    new URL("./tree_helpers.json", import.meta.url),
    JSON.stringify({ schemaVersion: 1, reId: reid, findNode: find }, null, 1) + "\n",
  );
});
