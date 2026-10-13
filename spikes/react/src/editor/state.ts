// Editor state = one reducer. The tree is the single source of truth; the DSL text is derived from it
// (except while the user is typing in the DSL view, where the text leads and the tree follows on
// every successful parse). Undo/redo snapshots cover all three views.
import type { DslError, Workout, WorkoutNode } from "@neo/shared";
import { backend } from "../backend";
import { allIds, remapSelection } from "./tree";

interface Snapshot { nodes: WorkoutNode[]; selection: string[] }

export interface Diff { before: WorkoutNode[]; after: WorkoutNode[]; changed: string[]; top: string[]; intent: string }

export interface EditorState {
  workout: Omit<Workout, "nodes">;
  nodes: WorkoutNode[];
  selection: string[];
  text: string;
  error: DslError | null;
  past: Snapshot[];
  future: Snapshot[];
  /** consecutive DSL keystrokes coalesce into one undo step */
  typing: boolean;
  diff: Diff | null;
}

export type EditorAction =
  | { type: "edit"; nodes: WorkoutNode[]; selection?: string[] }
  | { type: "dsl"; text: string }
  | { type: "select"; ids: string[] }
  | { type: "undo" }
  | { type: "redo" }
  | { type: "rename"; name: string }
  | { type: "load"; workout: Workout }
  | { type: "diff"; diff: Diff | null }
  | { type: "accept_diff" };

const LIMIT = 200;

export function initEditor(w: Workout): EditorState {
  const { nodes, ...meta } = w;
  return { workout: meta, nodes, selection: [], text: backend.printDsl(nodes), error: null, past: [], future: [], typing: false, diff: null };
}

function commit(s: EditorState, nodes: WorkoutNode[], selection: string[]): EditorState {
  return {
    ...s,
    nodes,
    selection,
    text: backend.printDsl(nodes),
    error: null,
    past: [...s.past, { nodes: s.nodes, selection: s.selection }].slice(-LIMIT),
    future: [],
    typing: false,
  };
}

export function editorReducer(s: EditorState, a: EditorAction): EditorState {
  switch (a.type) {
    case "edit":
      return commit(s, a.nodes, a.selection ?? s.selection.filter((id) => allIds(a.nodes).includes(id)));
    case "dsl": {
      const r = backend.parseDsl(a.text, s.workout.ftp);
      if ("error" in r) return { ...s, text: a.text, error: r.error };
      const selection = remapSelection(s.nodes, r.nodes, s.selection);
      const past = s.typing ? s.past : [...s.past, { nodes: s.nodes, selection: s.selection }].slice(-LIMIT);
      return { ...s, nodes: r.nodes, selection, text: a.text, error: null, past, future: [], typing: true };
    }
    case "select":
      return { ...s, selection: a.ids, typing: false };
    case "undo": {
      const prev = s.past[s.past.length - 1];
      if (!prev) return s;
      return { ...s, ...prev, text: backend.printDsl(prev.nodes), error: null, past: s.past.slice(0, -1), future: [{ nodes: s.nodes, selection: s.selection }, ...s.future], typing: false, diff: null };
    }
    case "redo": {
      const next = s.future[0];
      if (!next) return s;
      return { ...s, ...next, text: backend.printDsl(next.nodes), error: null, past: [...s.past, { nodes: s.nodes, selection: s.selection }], future: s.future.slice(1), typing: false, diff: null };
    }
    case "rename":
      return { ...s, workout: { ...s.workout, name: a.name } };
    case "load":
      return initEditor(a.workout);
    case "diff":
      return { ...s, diff: a.diff };
    case "accept_diff":
      return s.diff ? { ...commit(s, s.diff.after, s.diff.top), diff: null } : s;
  }
}
