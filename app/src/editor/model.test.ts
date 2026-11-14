// The editor model (DESIGN §5.7, #111, §10 defect #9): tree ops, undo/redo, DSL ↔ tree sync and invalid-draft
// refusal, on trees the core produced (editorFixture.ts answers only from the core's own answers).
import fc from "fast-check";
import { describe, expect, it } from "vitest";
import type { DslParse, WorkoutNode } from "../bindings";
import { DRAFTS, editorCommands } from "../views/editorFixture";
import {
  applied,
  canRedo,
  canUndo,
  insertAfter,
  load,
  nodeAt,
  parsed,
  pathOf,
  redo,
  removeNode,
  replaceNode,
  select,
  setLock,
  typed,
  undo,
  wrapNode,
  type EditorState,
} from "./model";

const FTP = 250;
const text = (name: string) => DRAFTS[name]!.text;
const ids = (nodes: readonly WorkoutNode[]): string[] =>
  nodes.flatMap((n) => [n.id, ...(n.type === "repeat" ? ids(n.children) : [])]);

/** A fresh editor on the core's parse of the `sweet-spot` draft. */
function fresh() {
  const core = editorCommands();
  const parse = (t: string) => core.parse_dsl(t, FTP);
  const first = parse(text("sweet-spot"));
  if (first.outcome !== "parsed") throw new Error("sweet-spot parses");
  const s = load(
    { id: "w", name: "Sweet spot", ftp: FTP, nodes: first.nodes },
    text("sweet-spot"),
  );
  /** Type `t` and hand the core's answer in, as the view does. */
  const type = (st: EditorState, t: string) =>
    parsed(typed(st, t), t, parse(t));
  return { core, parse, s, type };
}

describe("tree ops", () => {
  const { s } = fresh();
  const nodes = s.saved.nodes;
  const [warm, sets, cool] = nodes as [WorkoutNode, WorkoutNode, WorkoutNode];
  const step = (id: string): WorkoutNode => ({
    type: "step",
    id,
    label: null,
    end: { kind: "time", s: 60 },
    target: { kind: "pct_ftp", value: 50 },
    cadence: null,
  });

  it("remove keeps every other id and drops the subtree's", () => {
    const out = removeNode(nodes, sets.id);
    expect(ids(out)).toEqual([warm.id, cool.id]);
    const inner = removeNode(nodes, ids([sets])[1]!);
    expect(ids(inner)).toEqual(ids(nodes).filter((i) => i !== ids([sets])[1]));
    expect(inner[0]).toBe(warm); // untouched nodes are the same objects
  });

  it("replace puts the given nodes (their ids) in place of one", () => {
    const out = replaceNode(nodes, cool.id, [step("new-1"), step("new-2")]);
    expect(ids(out)).toEqual([...ids(nodes).slice(0, -1), "new-1", "new-2"]);
  });

  it("insert adds a node with its new id after another, or at the end", () => {
    expect(ids(insertAfter(nodes, warm.id, step("x")))[1]).toBe("x");
    expect(ids(insertAfter(nodes, null, step("x"))).at(-1)).toBe("x");
  });

  it("wrap keeps the wrapped node's id and gives the repeat a new one", () => {
    const out = wrapNode(nodes, warm.id, 2, "rep-1");
    expect(out[0]).toMatchObject({ type: "repeat", id: "rep-1", count: 2 });
    expect(out[0]!.type === "repeat" && out[0]!.children[0]).toBe(warm);
    expect(ids(out)).toEqual(["rep-1", ...ids(nodes)]);
  });

  it("an id not in the tree changes nothing", () => {
    expect(removeNode(nodes, "nope")).toEqual(nodes);
    expect(pathOf(nodes, "nope")).toBeNull();
    expect(nodeAt(nodes, [9])).toBeNull();
  });
});

describe("undo and redo", () => {
  it("a → b → c, undo, undo, redo returns to b, the same tree", () => {
    const { s: a, type } = fresh();
    const b = type(a, text("sweet-spot-4x"));
    const c = type(b, text("sweet-spot-4x-15m"));
    const back = redo(undo(undo(c)));
    expect(back.saved).toBe(b.saved);
    expect(back.saved.nodes).toBe(b.saved.nodes);
    expect(back.draft).toBe(text("sweet-spot-4x"));
    expect(undo(undo(c)).saved).toBe(a.saved);
    expect(canUndo(back) && canRedo(back)).toBe(true);
  });

  it("a new commit clears redo", () => {
    const { s: a, type } = fresh();
    const b = undo(type(a, text("sweet-spot-4x")));
    expect(canRedo(b)).toBe(true);
    expect(canRedo(type(b, text("sweet-spot-no-cool-down")))).toBe(false);
  });

  it("with nothing to undo or redo the state is unchanged", () => {
    const { s } = fresh();
    expect(undo(s)).toBe(s);
    expect(redo(s)).toBe(s);
  });
});

describe("DSL ↔ tree", () => {
  it("typing a valid text commits the core's tree with that text", () => {
    const { s, type, parse } = fresh();
    const next = type(s, text("sweet-spot-4x"));
    const want = parse(text("sweet-spot-4x"));
    if (want.outcome !== "parsed") throw new Error("parses");
    expect(next.saved.text).toBe(text("sweet-spot-4x"));
    expect(next.saved.nodes[1]).toMatchObject({ type: "repeat", count: 4 });
    expect(next.problem).toBeNull();
    expect(next.past).toEqual([s.saved]);
  });

  it("an invalid text keeps the saved state and shows the problem: no partial commit", () => {
    const { s, type } = fresh();
    const bad = type(s, text("missing-target"));
    expect(bad.saved).toBe(s.saved);
    expect(bad.past).toEqual([]);
    expect(bad.draft).toBe(text("missing-target"));
    expect(bad.problem?.message).toMatch(/power target/);
    // Undo drops the invalid draft with it; redo has nothing to bring back.
    expect(undo(bad)).toBe(bad);
    const { s: s2, type: type2 } = fresh();
    const after = type2(type2(s2, text("sweet-spot-4x")), text("implausible"));
    const undone = undo(after);
    expect(undone.saved).toBe(s2.saved);
    expect(undone.problem).toBeNull();
    expect(undone.draft).toBe(s2.saved.text);
  });

  it("an answer for a text that is no longer the draft is dropped", () => {
    const { s, parse } = fresh();
    const stale = parse(text("sweet-spot-4x"));
    const moved = typed(
      typed(s, text("sweet-spot-4x")),
      text("sweet-spot-4x-15m"),
    );
    expect(parsed(moved, text("sweet-spot-4x"), stale)).toBe(moved);
  });

  it("a tree op commits the tree with the core's canonical text", () => {
    const { s, core } = fresh();
    const nodes = removeNode(s.saved.nodes, s.saved.nodes[2]!.id);
    const next = applied(s, nodes, core.print_dsl(nodes));
    expect(next.saved.text).toBe(text("sweet-spot-no-cool-down"));
    expect(next.draft).toBe(next.saved.text);
    expect(undo(next).saved).toBe(s.saved);
  });
});

describe("selection", () => {
  it("survives undo and redo while its node exists, else collapses to null", () => {
    const { s, core } = fresh();
    const cool = s.saved.nodes[2]!.id;
    const warm = s.saved.nodes[0]!.id;
    const gone = removeNode(s.saved.nodes, cool);
    const b = applied(select(s, warm), gone, core.print_dsl(gone));
    expect(undo(b).selection).toBe(warm);
    expect(redo(undo(b)).selection).toBe(warm);
    const onCool = select(undo(b), cool);
    expect(onCool.selection).toBe(cool);
    expect(redo(onCool).selection).toBeNull();
  });

  it("follows its path through a text commit, which gives every node a fresh id", () => {
    const { s, type } = fresh();
    const sel = select(s, s.saved.nodes[1]!.id);
    const next = type(sel, text("sweet-spot-4x"));
    expect(next.selection).toBe(next.saved.nodes[1]!.id);
    expect(next.selection).not.toBe(sel.selection);
    const shorter = type(
      select(s, s.saved.nodes[2]!.id),
      text("sweet-spot-no-cool-down"),
    );
    expect(shorter.selection).toBeNull();
  });

  it("an id not in the tree selects nothing", () => {
    expect(select(fresh().s, "nope").selection).toBeNull();
  });
});

describe("lock (#112)", () => {
  it("refuses every edit, undo, redo and selection while set", () => {
    const { s, type, core } = fresh();
    const b = type(s, text("sweet-spot-4x"));
    const locked = setLock(b, { kind: "pending" });
    expect(typed(locked, "x")).toBe(locked);
    expect(
      parsed(locked, locked.draft, core.parse_dsl(text("sweet-spot"), FTP)),
    ).toBe(locked);
    expect(applied(locked, [], "")).toBe(locked);
    expect(undo(locked)).toBe(locked);
    expect(redo(locked)).toBe(locked);
    expect(select(locked, null)).toBe(locked);
    expect(canUndo(locked)).toBe(false);
    expect(undo(setLock(locked, null)).saved).toBe(s.saved);
  });
});

// ---- properties (fast-check) ----------------------------------------------------------------------------------------

const DRAFT_NAMES = Object.keys(DRAFTS);
type Op =
  | { op: "type"; name: string }
  | { op: "remove"; at: number }
  | { op: "wrap"; at: number }
  | { op: "select"; at: number }
  | { op: "undo" }
  | { op: "redo" };
const opArb: fc.Arbitrary<Op> = fc.oneof(
  fc.record({
    op: fc.constant("type" as const),
    name: fc.constantFrom(...DRAFT_NAMES),
  }),
  fc.record({ op: fc.constant("remove" as const), at: fc.nat(8) }),
  fc.record({ op: fc.constant("wrap" as const), at: fc.nat(8) }),
  fc.record({ op: fc.constant("select" as const), at: fc.nat(8) }),
  fc.record({ op: fc.constant("undo" as const) }),
  fc.record({ op: fc.constant("redo" as const) }),
);

/** Run `ops`; a tree op the fixture has no printed text for is skipped (the core's answer is unknown). */
function run(
  ops: readonly Op[],
  check: (before: EditorState, after: EditorState, op: Op) => void,
) {
  const { s, type, core } = fresh();
  let st = s;
  let wraps = 0;
  for (const op of ops) {
    const all = ids(st.saved.nodes);
    let next = st;
    if (op.op === "type") next = type(st, DRAFTS[op.name]!.text);
    else if (op.op === "undo") next = undo(st);
    else if (op.op === "redo") next = redo(st);
    else if (op.op === "select")
      next = select(st, all[op.at % (all.length + 1)] ?? null);
    else if (all.length > 0) {
      const id = all[op.at % all.length]!;
      const nodes =
        op.op === "remove"
          ? removeNode(st.saved.nodes, id)
          : wrapNode(st.saved.nodes, id, 2, `wrap-${++wraps}`);
      let printed: string | null = null;
      try {
        printed = core.print_dsl(nodes);
      } catch {
        printed = null;
      }
      if (printed !== null) next = applied(st, nodes, printed);
    }
    check(st, next, op);
    st = next;
  }
}

describe("properties", () => {
  it("the selection is always null or a node of the saved tree", () => {
    fc.assert(
      fc.property(fc.array(opArb, { maxLength: 30 }), (ops) =>
        run(ops, (_, after) => {
          if (after.selection !== null)
            expect(pathOf(after.saved.nodes, after.selection)).not.toBeNull();
        }),
      ),
    );
  });

  it("undo then redo returns the same saved state; history length is conserved", () => {
    fc.assert(
      fc.property(fc.array(opArb, { maxLength: 30 }), (ops) =>
        run(ops, (_, after) => {
          const total = after.past.length + after.future.length;
          if (canUndo(after)) {
            const u = undo(after);
            expect(redo(u).saved).toBe(after.saved);
            expect(u.past.length + u.future.length).toBe(total);
          }
        }),
      ),
    );
  });

  it("a saved state always holds a tree and the text it was committed with; only an invalid draft differs", () => {
    fc.assert(
      fc.property(fc.array(opArb, { maxLength: 30 }), (ops) =>
        run(ops, (before, after, op) => {
          if (after.problem === null)
            expect(after.draft).toBe(after.saved.text);
          // A problem comes only from typing, with the saved state untouched; any other step keeps the draft and its
          // problem as they were (an undo or redo that moves drops both).
          if (after.problem !== null) {
            expect(after.saved).toBe(before.saved);
            if (op.op !== "type") {
              expect(after.problem).toBe(before.problem);
              expect(after.draft).toBe(before.draft);
            }
          }
        }),
      ),
    );
  });

  it("remove keeps every id outside the removed subtree; wrap keeps every id and adds one", () => {
    fc.assert(
      fc.property(fc.constantFrom(...DRAFT_NAMES), fc.nat(8), (name, at) => {
        const answer: DslParse = editorCommands().parse_dsl(
          DRAFTS[name]!.text,
          FTP,
        );
        if (answer.outcome !== "parsed") return;
        const all = ids(answer.nodes);
        if (all.length === 0) return;
        const id = all[at % all.length]!;
        const path = pathOf(answer.nodes, id)!;
        const gone = ids([nodeAt(answer.nodes, path)!]);
        expect(ids(removeNode(answer.nodes, id))).toEqual(
          all.filter((i) => !gone.includes(i)),
        );
        const wrapped = ids(wrapNode(answer.nodes, id, 3, "fresh"));
        expect(wrapped.filter((i) => i !== "fresh")).toEqual(all);
        expect(wrapped.filter((i) => i === "fresh")).toHaveLength(1);
      }),
    );
  });
});
