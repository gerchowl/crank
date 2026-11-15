// The workout editor's model (DESIGN §5.7, #111; fixes §10 defect #9): pure functions over one state that the three
// views (profile, outline, text) render, so every edit, undo and redo goes through here and is unit-tested. It holds
// no I/O: the DSL parser and printer are the core's (`parse_dsl`, `print_dsl`, docs/DSL.md); the view calls them and
// hands their answer in (`typed`, `parsed`, `applied`). Port of design/prototype/src/editor/model.ts.
//
// - The saved state is the workout tree plus the text it was committed with; the text view's `draft` may run ahead
//   of it. A draft that does not parse leaves the saved state untouched and carries the core's problem (no partial
//   commit).
// - History holds saved states only: undo and redo restore a tree with its text, so they need no printer.
// - Undo groups (DESIGN §5.7): the text commits of one burst of typing are one undo step. A burst ends at a pause of
//   `GROUP_MS` between two keystrokes, or at any other commit, undo or redo; a tree op is a group of its own. The
//   history keeps the last `HISTORY_LIMIT` steps and drops the oldest past it. Both numbers are prosemirror-history's
//   defaults (`newGroupDelay`, `depth`), the convention text editors follow; time is handed in, so the model stays pure.
// - The selection is a node id. It survives a commit, undo or redo while that id is in the tree; a text commit gives
//   every node a fresh id (the core's parser), so the selection follows its path there (docs/DSL.md, "Tree-helper
//   ownership"); otherwise it collapses to null.
// - `lock` is the AI rewrite's (#112): while it is set every edit, undo, redo and selection change is refused.
import type { DslParse, DslProblem, Workout, WorkoutNode } from "../bindings";

/** A pending AI rewrite (#112 owns its contents): while one is set the editor refuses every edit. */
/** A pause this long (ms) between two keystrokes ends an undo group (prosemirror-history's `newGroupDelay`). */
export const GROUP_MS = 500;
/** The most undo steps kept; past it the oldest is dropped (prosemirror-history's `depth`). */
export const HISTORY_LIMIT = 100;

/** The burst of typing in progress: when its last keystroke was, and whether it has its undo step already. */
export interface Typing {
  readonly at: number;
  readonly open: boolean;
}

export interface Lock {
  readonly kind: "pending" | "diff";
}

/** One saved state of the workout: its tree and the text it was committed with. */
export interface Saved {
  readonly nodes: readonly WorkoutNode[];
  readonly text: string;
}

export interface EditorState {
  /** The workout's identity and reference FTP; its `nodes` are `saved.nodes`. */
  readonly workout: Omit<Workout, "nodes">;
  readonly saved: Saved;
  /** The text view's content: `saved.text`, or a draft typed since. */
  readonly draft: string;
  /** Why the draft does not parse; null while it does or is not yet parsed. */
  readonly problem: DslProblem | null;
  /** The selected node's id. */
  readonly selection: string | null;
  readonly past: readonly Saved[];
  readonly future: readonly Saved[];
  /** The typing burst whose commits share one undo step; null after any other commit, undo or redo. */
  readonly typing: Typing | null;
  readonly lock: Lock | null;
}

/** A path of child indices from the root list to a node. */
export type Path = readonly number[];

export function load(workout: Workout, text: string): EditorState {
  const { nodes, ...rest } = workout;
  return {
    workout: rest,
    saved: { nodes, text },
    draft: text,
    problem: null,
    selection: null,
    past: [],
    future: [],
    typing: null,
    lock: null,
  };
}

/** The workout as saved, for compiling and riding. */
export const savedWorkout = (s: EditorState): Workout => ({
  ...s.workout,
  nodes: [...s.saved.nodes],
});
export const canUndo = (s: EditorState) => !s.lock && s.past.length > 0;
export const canRedo = (s: EditorState) => !s.lock && s.future.length > 0;

// ---- tree ops (pure; ids of untouched nodes are kept) ------------------------------------------------------------

/** The path of the node with `id`, or null. */
export function pathOf(nodes: readonly WorkoutNode[], id: string): Path | null {
  for (const [i, n] of nodes.entries()) {
    if (n.id === id) return [i];
    if (n.type === "repeat") {
      const p = pathOf(n.children, id);
      if (p) return [i, ...p];
    }
  }
  return null;
}
/** The node at `path`, or null. */
export function nodeAt(
  nodes: readonly WorkoutNode[],
  path: Path,
): WorkoutNode | null {
  const [i, ...rest] = path;
  const n = i === undefined ? undefined : nodes[i];
  if (!n) return null;
  if (rest.length === 0) return n;
  return n.type === "repeat" ? nodeAt(n.children, rest) : null;
}
const has = (nodes: readonly WorkoutNode[], id: string | null) =>
  id !== null && pathOf(nodes, id) !== null;

/** Replace the list that holds `id` by `edit(list, index)`; the tree is unchanged when `id` is not in it. */
function editAround(
  nodes: readonly WorkoutNode[],
  id: string,
  edit: (list: readonly WorkoutNode[], i: number) => WorkoutNode[],
): WorkoutNode[] {
  const i = nodes.findIndex((n) => n.id === id);
  if (i >= 0) return edit(nodes, i);
  return nodes.map((n) =>
    n.type === "repeat" && has(n.children, id)
      ? { ...n, children: editAround(n.children, id, edit) }
      : n,
  );
}
/** Remove the node with `id` (and its subtree). */
export const removeNode = (nodes: readonly WorkoutNode[], id: string) =>
  editAround(nodes, id, (l, i) => [...l.slice(0, i), ...l.slice(i + 1)]);
/** Replace the node with `id` by `by` (an AI rewrite's proposal keeps the ids it was given). */
export const replaceNode = (
  nodes: readonly WorkoutNode[],
  id: string,
  by: readonly WorkoutNode[],
) =>
  editAround(nodes, id, (l, i) => [...l.slice(0, i), ...by, ...l.slice(i + 1)]);
/** Insert `node` right after the node with `id`, or at the end when `id` is null. */
export const insertAfter = (
  nodes: readonly WorkoutNode[],
  id: string | null,
  node: WorkoutNode,
) =>
  id === null
    ? [...nodes, node]
    : editAround(nodes, id, (l, i) => [
        ...l.slice(0, i + 1),
        node,
        ...l.slice(i + 1),
      ]);
/** Wrap the node with `id` in a new repeat of `count` with the fresh id `repeatId`; the node keeps its id. */
export const wrapNode = (
  nodes: readonly WorkoutNode[],
  id: string,
  count: number,
  repeatId: string,
) =>
  editAround(nodes, id, (l, i) => [
    ...l.slice(0, i),
    {
      type: "repeat",
      id: repeatId,
      label: null,
      count,
      children: [l[i]!],
      progression: null,
      skipLastRest: null,
    },
    ...l.slice(i + 1),
  ]);

// ---- editor transitions -------------------------------------------------------------------------------------------

/**
 * Commit `nodes` with `text` as the new saved state and clear redo. Unless `grouped` (a later commit of the same typing
 * burst, whose undo step exists), the old state becomes one more undo step, the oldest dropped past `HISTORY_LIMIT`.
 */
function commit(
  s: EditorState,
  nodes: readonly WorkoutNode[],
  text: string,
  selection: string | null,
  grouped: boolean,
): EditorState {
  return {
    ...s,
    saved: { nodes, text },
    draft: text,
    problem: null,
    selection: has(nodes, selection) ? selection : null,
    past: grouped ? s.past : [...s.past, s.saved].slice(-HISTORY_LIMIT),
    future: [],
  };
}

/**
 * The rider typed `text` at time `at` (ms, any monotonic clock): the draft follows at once; its parse answer arrives
 * with `parsed`. A keystroke within `GROUP_MS` of the previous one continues the burst, else it starts a new one.
 */
export function typed(s: EditorState, text: string, at: number): EditorState {
  if (s.lock) return s;
  const open =
    s.typing !== null && at - s.typing.at < GROUP_MS && s.typing.open;
  return { ...s, draft: text, typing: { at, open } };
}

/**
 * The core's answer for parsing `text`. An answer for a text that is no longer the draft is stale and dropped; an
 * invalid one keeps the saved state and shows the problem; a parsed one is committed with the draft as its text, the
 * selection following its path, into the typing burst's undo step. A text equal to the saved one commits nothing
 * (the tree is the same).
 */
export function parsed(
  s: EditorState,
  text: string,
  answer: DslParse,
): EditorState {
  if (s.lock || text !== s.draft) return s;
  if (answer.outcome === "invalid") return { ...s, problem: answer.problem };
  if (text === s.saved.text) return { ...s, problem: null };
  const at = s.selection === null ? null : pathOf(s.saved.nodes, s.selection);
  const follow = at === null ? null : (nodeAt(answer.nodes, at)?.id ?? null);
  const grouped = s.typing?.open ?? false;
  const next = commit(s, answer.nodes, text, follow, grouped);
  return { ...next, typing: s.typing && { ...s.typing, open: true } };
}

/** A tree op's result `nodes` with the core's canonical `text` for it (`print_dsl`), committed. */
export function applied(
  s: EditorState,
  nodes: readonly WorkoutNode[],
  text: string,
): EditorState {
  if (s.lock) return s;
  return { ...commit(s, nodes, text, s.selection, false), typing: null };
}

export function select(s: EditorState, id: string | null): EditorState {
  if (s.lock) return s;
  return { ...s, selection: id === null || has(s.saved.nodes, id) ? id : null };
}

/** Back to the previous saved state; a draft that does not parse is dropped with the state it was typed over. */
export function undo(s: EditorState): EditorState {
  const prev = s.past.at(-1);
  if (s.lock || !prev) return s;
  return restore(s, prev, s.past.slice(0, -1), [s.saved, ...s.future]);
}
export function redo(s: EditorState): EditorState {
  const next = s.future[0];
  if (s.lock || !next) return s;
  return restore(s, next, [...s.past, s.saved], s.future.slice(1));
}
function restore(
  s: EditorState,
  to: Saved,
  past: readonly Saved[],
  future: readonly Saved[],
): EditorState {
  return {
    ...s,
    saved: to,
    draft: to.text,
    problem: null,
    selection: has(to.nodes, s.selection) ? s.selection : null,
    past,
    future,
    typing: null,
  };
}

/** Set or clear the AI rewrite's lock (#112). */
export const setLock = (s: EditorState, lock: Lock | null): EditorState => ({
  ...s,
  lock,
});
