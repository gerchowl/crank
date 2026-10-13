// Editor model: a pure-data state machine behind the three synced views (DESIGN §5.7).
// #1: while an AI rewrite is pending or its diff is open, every edit is refused, and accept
// re-verifies that the target node is unchanged (id + content) before applying.
// #9: production keeps this boundary and unit-tests it; the prototype keeps it testable.
import { demoWorkout, type DslError, type RewriteIntent, type Workout, type WorkoutNode } from "@crank/shared";
import { backend, settle } from "../session";
import { createStore } from "../store";

export type Lock =
  | { kind: "pending"; path: number; baseHash: string; intent: RewriteIntent; token: number }
  | { kind: "diff"; path: number; baseHash: string; intent: RewriteIntent; token: number; proposal: WorkoutNode[] };

export interface EditorState {
  workout: Workout;
  text: string;
  error: DslError | null;
  selection: number | null; // top-level index (the prototype edits top-level items)
  past: WorkoutNode[][];
  future: WorkoutNode[][];
  lock: Lock | null;
  notice: string | null;
}

const initial = demoWorkout(backend.athlete.ftp);
export const editorStore = createStore<EditorState>({
  workout: initial,
  text: backend.printDsl(initial.nodes),
  error: null,
  selection: null,
  past: [],
  future: [],
  lock: null,
  notice: null,
});

const hashOf = (n: WorkoutNode | undefined) => (n ? `${n.id}:${backend.printDsl([n])}` : "");
let tokens = 0;

function commit(s: EditorState, nodes: WorkoutNode[], text = backend.printDsl(nodes)): EditorState {
  return { ...s, workout: { ...s.workout, nodes }, text, error: null, past: [...s.past, s.workout.nodes], future: [], notice: null };
}

export const editor = {
  load(workout: Workout) {
    editorStore.set((s) => (s.lock ? s : { ...s, workout, text: backend.printDsl(workout.nodes), error: null, selection: null, past: [], future: [], notice: null }));
  },
  setText(text: string) {
    editorStore.set((s) => {
      if (s.lock) return s;
      const r = backend.parseDsl(text, s.workout.ftp);
      return "error" in r ? { ...s, text, error: r.error } : commit(s, r.nodes, text);
    });
  },
  select(i: number | null) {
    editorStore.set((s) => (s.lock ? s : { ...s, selection: i }));
  },
  undo() {
    editorStore.set((s) => {
      const prev = s.past.at(-1);
      if (s.lock || !prev) return s;
      return { ...s, workout: { ...s.workout, nodes: prev }, text: backend.printDsl(prev), error: null, past: s.past.slice(0, -1), future: [s.workout.nodes, ...s.future] };
    });
  },
  redo() {
    editorStore.set((s) => {
      const next = s.future[0];
      if (s.lock || !next) return s;
      return { ...s, workout: { ...s.workout, nodes: next }, text: backend.printDsl(next), error: null, past: [...s.past, s.workout.nodes], future: s.future.slice(1) };
    });
  },
  requestRewrite(intent: RewriteIntent) {
    const s = editorStore.get();
    const path = s.selection;
    const node = path === null ? undefined : s.workout.nodes[path];
    if (s.lock || path === null || !node || s.error) return;
    const token = ++tokens;
    editorStore.set({ ...s, lock: { kind: "pending", path, baseHash: hashOf(node), intent, token }, notice: null });
    void settle(backend.planner.rewrite([node], intent)).then((proposal) => {
      editorStore.set((cur) => (cur.lock?.kind === "pending" && cur.lock.token === token ? { ...cur, lock: { ...cur.lock, kind: "diff", proposal } } : cur));
    });
  },
  accept() {
    const s = editorStore.get();
    if (s.lock?.kind !== "diff") return;
    const { path, baseHash, proposal, intent } = s.lock;
    if (hashOf(s.workout.nodes[path]) !== baseHash) {
      editorStore.set({ ...s, lock: null, notice: "The selection changed since the request; asking again." });
      editor.requestRewrite(intent);
      return;
    }
    const nodes = [...s.workout.nodes.slice(0, path), ...proposal, ...s.workout.nodes.slice(path + 1)];
    editorStore.set({ ...commit(s, nodes), lock: null });
  },
  reject() {
    editorStore.set((s) => ({ ...s, lock: null }));
  },
  /** The workout as it would be after accepting the open diff (for the ghost overlay). */
  proposed(s: EditorState): Workout | null {
    if (s.lock?.kind !== "diff") return null;
    const { path, proposal } = s.lock;
    return { ...s.workout, nodes: [...s.workout.nodes.slice(0, path), ...proposal, ...s.workout.nodes.slice(path + 1)] };
  },
};
