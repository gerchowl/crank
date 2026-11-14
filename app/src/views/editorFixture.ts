// The core's editor commands (`parse_dsl`, `print_dsl`, `compile_workout`, #111) for the test backends, answered only
// from the core's own answers: crates/crank-core/tests/fixtures/editor.json, which the `editor_fixture` test checks
// against the core (`just editor-fixture` rewrites it). There is no DSL parser in the app, so a mock cannot parse; it
// looks the draft up, and a draft not in the file is a broken test (a thrown `Error`), never a guessed answer.
//
// Refusals, as the core gives them (crates/crank-core/src):
// - `parse_dsl` answers `invalid` with the problem for a text that does not parse (core.rs `Core::parse_dsl`); its one
//   refusal, `internal` for a span past u32 (dto.rs `DslProblem`), is unreachable from a page and not modelled.
// - `print_dsl` never fails (core.rs `Core::print_dsl`).
// - `compile_workout` refuses a tree the domain cannot compile with `invalid_input` (core.rs `Core::compile_workout`,
//   error.rs `ApiError` and its `From<CompileError>`): the file's `refused`. Its `internal` for a block index past u32
//   (dto.rs `WorkoutBlock`) is unreachable from a page and not modelled.
// - Tauri itself rejects an argument that does not deserialise, with a plain string, before the core runs: the bindings
//   type an `f64` as `number | null` (serde writes NaN as null), but a `null` FTP is no `f64` (`parse_dsl`'s `ftp`,
//   `compile_workout`'s `workout.ftp`).
import type {
  ApiError,
  CompiledView,
  DslParse,
  DslProblem,
  Workout,
  WorkoutNode,
} from "../bindings";
import drafts from "../../../crates/crank-core/tests/fixtures/editor.json";

/** One draft text and the core's answers for it. */
export type Draft = { text: string; ftp: number } & (
  | { problem: DslProblem }
  | ({ nodes: WorkoutNode[]; printed: string } & (
      { compiled: CompiledView } | { refused: ApiError }
    ))
);
/** The drafts by name (the `DRAFTS` of crates/crank-core/tests/editor_fixture.rs). */
export const DRAFTS = drafts as unknown as Record<string, Draft>;

/** Every node id of a tree, in tree order (the order the fixture numbers them). */
function ids(nodes: readonly WorkoutNode[]): string[] {
  return nodes.flatMap((n) => [
    n.id,
    ...(n.type === "repeat" ? ids(n.children) : []),
  ]);
}
/** A tree's structure without its ids, with sorted keys: equal trees give equal strings. */
function shape(nodes: readonly WorkoutNode[]): string {
  return JSON.stringify(nodes, (key, value: unknown) => {
    if (key === "id") return undefined;
    if (value && typeof value === "object" && !Array.isArray(value)) {
      const o = value as Record<string, unknown>;
      return Object.fromEntries(
        Object.keys(o)
          .sort()
          .map((k) => [k, o[k]]),
      );
    }
    return value;
  });
}
/** Replace every id of `nodes` with the next one of `next`. */
function reId(
  nodes: readonly WorkoutNode[],
  next: () => string,
): WorkoutNode[] {
  return nodes.map((n) =>
    n.type === "repeat"
      ? { ...n, id: next(), children: reId(n.children, next) }
      : { ...n, id: next() },
  );
}
const parsed = (d: Draft) => ("nodes" in d ? d : null);
/** The draft whose tree is `nodes` (and whose FTP is `ftp`, when given). */
function byTree(nodes: readonly WorkoutNode[], ftp?: number) {
  const s = shape(nodes);
  const d = Object.values(DRAFTS)
    .map(parsed)
    .find(
      (p) => p && shape(p.nodes) === s && (ftp === undefined || p.ftp === ftp),
    );
  if (!d)
    throw new Error(
      `fixture: no draft of editor.json has this tree (add its text to editor_fixture.rs): ${s}`,
    );
  return d;
}

/** The core's editor commands, with a fresh `dsl-<n>` id per parsed node as the core gives (core.rs `Core::parse_dsl`). */
export function editorCommands() {
  let n = 0;
  const fresh = () => `dsl-${++n}`;
  return {
    parse_dsl(text: string, ftp: number | null): DslParse {
      if (ftp === null)
        throw "invalid args `ftp` for command `parse_dsl`: invalid type: null, expected f64";
      const d = Object.values(DRAFTS).find(
        (x) => x.text === text && x.ftp === ftp,
      );
      if (!d)
        throw new Error(
          `fixture: ${JSON.stringify(text)} at ${ftp} W is not in editor.json (add it to editor_fixture.rs)`,
        );
      return "problem" in d
        ? { outcome: "invalid", problem: d.problem }
        : { outcome: "parsed", nodes: reId(d.nodes, fresh) };
    },
    print_dsl(nodes: WorkoutNode[]): string {
      return byTree(nodes).printed;
    },
    compile_workout(workout: Workout): CompiledView {
      if (workout.ftp === null)
        throw "invalid args `workout` for command `compile_workout`: invalid type: null, expected f64";
      const d = byTree(workout.nodes, workout.ftp);
      if ("refused" in d) throw d.refused;
      // The file's compiled view names the file's ids; give it the request's.
      const theirs = ids(d.nodes);
      const ours = ids(workout.nodes);
      const map = new Map(theirs.map((id, i) => [id, ours[i]]));
      const to = (id: string) => map.get(id) ?? id;
      return {
        ...d.compiled,
        blocks: d.compiled.blocks.map((b) => ({ ...b, nodeId: to(b.nodeId) })),
        issues: d.compiled.issues.map((i) => ({
          ...i,
          nodeId: i.nodeId === null ? null : to(i.nodeId),
        })),
      };
    },
  };
}
