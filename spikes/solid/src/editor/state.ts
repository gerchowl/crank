// Editor model: the workout lives in a store (reconciled by node id, so outline rows are
// stable), every edit goes through commit() → one undo step, and the DSL text is derived
// from the tree except while the user is typing in it.
import { batch, createMemo, createResource, createRoot, createSignal } from "solid-js";
import { createStore, reconcile, unwrap } from "solid-js/store";
import { demoWorkout, type RewriteIntent, type Workout, type WorkoutNode } from "@neo/shared";
import { backend } from "../backend";
import { allIds, carryIds, duplicate, insertAt, move, removeIds, replaceRange, selectedNodes, siblingRange, unwrap as unwrapNode, updateNode, wrap } from "./tree";

type Source = "text" | "tree";

interface HistoryEntry {
  nodes: WorkoutNode[];
  key: string | null;
  at: number;
}

const SNIPPET_KEY = "neo-trainer.snippets.v1";
const COALESCE_MS = 1200;

function loadSnippets(): { name: string; nodes: WorkoutNode[] }[] {
  try {
    const raw = localStorage.getItem(SNIPPET_KEY);
    const v: unknown = raw ? JSON.parse(raw) : [];
    return Array.isArray(v) ? (v as { name: string; nodes: WorkoutNode[] }[]) : [];
  } catch {
    return [];
  }
}

export interface RewriteRequest {
  intent: RewriteIntent;
  ids: string[];
  nodes: WorkoutNode[];
  seq: number;
}

export const editor = createRoot(() => {
  const seed = backend.ride.current()?.workout ?? demoWorkout(backend.athlete.ftp);
  const [doc, setDoc] = createStore<Workout>(structuredClone(seed));
  const [selection, setSelection] = createSignal<ReadonlySet<string>>(new Set());
  const [text, setText] = createSignal(backend.printDsl(seed.nodes));
  const [textError, setTextError] = createSignal<{ message: string; offset: number; length: number } | null>(null);
  const [collapsed, setCollapsed] = createSignal<ReadonlySet<string>>(new Set());
  const [past, setPast] = createSignal<HistoryEntry[]>([]);
  const [future, setFuture] = createSignal<WorkoutNode[][]>([]);
  const [userSnippets, setUserSnippets] = createSignal(loadSnippets());

  const nodes = () => doc.nodes;
  const snapshot = (): WorkoutNode[] => structuredClone(unwrap(doc.nodes));

  /** The one way to change the tree. `key` coalesces bursts (typing, dragging) into one undo step. */
  const commit = (next: WorkoutNode[], opts: { source?: Source; key?: string } = {}) => {
    const now = performance.now();
    batch(() => {
      const hist = past();
      const top = hist[hist.length - 1];
      if (!(opts.key && top?.key === opts.key && now - top.at < COALESCE_MS)) {
        setPast([...hist.slice(-199), { nodes: snapshot(), key: opts.key ?? null, at: now }]);
      } else {
        setPast([...hist.slice(0, -1), { ...top, at: now }]);
      }
      setFuture([]);
      setDoc("nodes", reconcile(next, { key: "id" }));
      if (opts.source !== "text") {
        setText(backend.printDsl(next));
        setTextError(null);
      }
      // drop selection entries that no longer exist
      const ids = allIds(next);
      const sel = [...selection()].filter((id) => ids.has(id));
      if (sel.length !== selection().size) setSelection(new Set(sel));
    });
  };

  const restore = (n: WorkoutNode[]) => {
    setDoc("nodes", reconcile(n, { key: "id" }));
    setText(backend.printDsl(n));
    setTextError(null);
  };

  const compiled = createMemo(() => backend.compileWorkout(structuredClone(unwrap({ ...doc, nodes: doc.nodes }))));

  // ---------- AI rewrite (createResource keyed on the request) ----------
  const [rewriteReq, setRewriteReq] = createSignal<RewriteRequest | null>(null);
  let seq = 0;
  const [rewrite] = createResource(rewriteReq, async (req) => {
    const out = await backend.planner.rewrite(req.nodes, req.intent);
    const before = snapshot();
    const after = replaceRange(before, new Set(req.ids), out);
    return { req, before, after: after ?? before, changed: allIds(out), top: out.map((n) => n.id) };
  });
  const diff = () => {
    const r = rewrite();
    const req = rewriteReq();
    return r && req && r.req.seq === req.seq ? r : null;
  };

  /** What the profile/summary show: the AI proposal while a diff is open, else the document. */
  const preview = createMemo(() => {
    const d = diff();
    return d ? backend.compileWorkout({ ...unwrap(doc), nodes: d.after }) : compiled();
  });

  const actions = {
    commit,
    select(id: string, additive: boolean) {
      if (!additive) {
        setSelection(new Set([id]));
        return;
      }
      const s = new Set(selection());
      if (s.has(id)) s.delete(id);
      else s.add(id);
      setSelection(s);
    },
    selectOnly(ids: string[]) {
      setSelection(new Set(ids));
    },
    clearSelection() {
      setSelection(new Set<string>());
    },
    toggleCollapsed(id: string) {
      const s = new Set(collapsed());
      if (s.has(id)) s.delete(id);
      else s.add(id);
      setCollapsed(s);
    },
    setText(t: string) {
      setText(t);
      const r = backend.parseDsl(t, doc.ftp);
      if ("error" in r) {
        setTextError(r.error);
        return;
      }
      setTextError(null);
      commit(carryIds(snapshot(), r.nodes), { source: "text", key: "text" });
    },
    undo() {
      const hist = past();
      const prev = hist[hist.length - 1];
      if (!prev) return;
      batch(() => {
        setFuture([snapshot(), ...future()]);
        setPast(hist.slice(0, -1));
        restore(prev.nodes);
      });
    },
    redo() {
      const [next, ...rest] = future();
      if (!next) return;
      batch(() => {
        setPast([...past(), { nodes: snapshot(), key: null, at: performance.now() }]);
        setFuture(rest);
        restore(next);
      });
    },
    wrap() {
      const id = `r-${Math.random().toString(36).slice(2, 8)}`;
      const out = wrap(snapshot(), selection(), id);
      if (out) {
        commit(out);
        setSelection(new Set([id]));
      }
    },
    unwrap() {
      const [id] = [...selection()];
      if (!id) return;
      const out = unwrapNode(snapshot(), id);
      if (out) commit(out);
    },
    duplicate() {
      const out = duplicate(snapshot(), selection());
      if (out) {
        commit(out.nodes);
        setSelection(new Set(out.newIds));
      }
    },
    remove() {
      if (!selection().size) return;
      commit(removeIds(snapshot(), selection()));
      setSelection(new Set<string>());
    },
    update(id: string, fn: (n: WorkoutNode) => WorkoutNode, key?: string) {
      commit(updateNode(snapshot(), id, fn), key ? { key } : {});
    },
    move(id: string, anchorId: string, where: "before" | "after" | "into") {
      const out = move(snapshot(), id, { anchorId, where });
      if (out) commit(out);
    },
    insertNodes(items: WorkoutNode[]) {
      const cur = snapshot();
      const r = siblingRange(cur, selection());
      const parent = r?.parent ?? [];
      const index = r ? r.to + 1 : cur.length;
      commit(insertAt(cur, parent, index, items));
      setSelection(new Set(items.map((n) => n.id)));
    },
    saveSnippet(name: string) {
      const sel = selectedNodes(snapshot(), selection());
      if (!sel.length) return;
      const next = [...userSnippets(), { name, nodes: structuredClone(sel) }];
      setUserSnippets(next);
      try {
        localStorage.setItem(SNIPPET_KEY, JSON.stringify(next));
      } catch {
        /* ignore */
      }
    },
    requestRewrite(intent: RewriteIntent) {
      const sel = selectedNodes(snapshot(), selection());
      if (!sel.length) return;
      setRewriteReq({ intent, ids: [...selection()], nodes: structuredClone(sel), seq: ++seq });
    },
    acceptRewrite() {
      const d = diff();
      if (!d) return;
      batch(() => {
        commit(d.after);
        setSelection(new Set(d.top));
        setRewriteReq(null);
      });
    },
    rejectRewrite() {
      setRewriteReq(null);
    },
    load(w: Workout) {
      batch(() => {
        setDoc(reconcile(structuredClone(w)));
        setText(backend.printDsl(w.nodes));
        setTextError(null);
        setPast([]);
        setFuture([]);
        setSelection(new Set<string>());
        setRewriteReq(null);
      });
    },
    rename(name: string) {
      setDoc("name", name);
    },
  };

  return {
    doc,
    nodes,
    selection,
    text,
    textError,
    collapsed,
    compiled,
    preview,
    canUndo: () => past().length > 0,
    canRedo: () => future().length > 0,
    snippets: () => [...backend.snippets(), ...userSnippets()],
    rewrite,
    rewriteReq,
    diff,
    actions,
  };
});
