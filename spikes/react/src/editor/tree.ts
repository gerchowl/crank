// Pure, immutable edits on the workout tree. Nodes are addressed by id for user intent and by
// path (child indices) for structure; ids are re-derived from paths after a DSL re-parse.
import { reId, type RepeatNode, type StepNode, type WorkoutNode } from "@neo/shared";

export type Path = number[];

export function pathOf(nodes: readonly WorkoutNode[], id: string, base: Path = []): Path | null {
  for (let i = 0; i < nodes.length; i++) {
    const n = nodes[i];
    if (!n) continue;
    if (n.id === id) return [...base, i];
    if (n.type === "repeat") {
      const p = pathOf(n.children, id, [...base, i]);
      if (p) return p;
    }
  }
  return null;
}

export function nodeAt(nodes: readonly WorkoutNode[], path: Path): WorkoutNode | undefined {
  let list: readonly WorkoutNode[] = nodes;
  let cur: WorkoutNode | undefined;
  for (const i of path) {
    cur = list[i];
    if (!cur) return undefined;
    list = cur.type === "repeat" ? cur.children : [];
  }
  return cur;
}

/** Replace the child list at `parent` (root = []). */
function withChildren(nodes: readonly WorkoutNode[], parent: Path, f: (children: WorkoutNode[]) => WorkoutNode[]): WorkoutNode[] {
  if (parent.length === 0) return f([...nodes]);
  const [i, ...rest] = parent;
  return nodes.map((n, k) => (k === i && n.type === "repeat" ? { ...n, children: withChildren(n.children, rest, f) } : n));
}

export function updateNode(nodes: readonly WorkoutNode[], id: string, f: (n: WorkoutNode) => WorkoutNode): WorkoutNode[] {
  return nodes.map((n) => (n.id === id ? f(n) : n.type === "repeat" ? { ...n, children: updateNode(n.children, id, f) } : n));
}

export function removeIds(nodes: readonly WorkoutNode[], ids: ReadonlySet<string>): WorkoutNode[] {
  return nodes.filter((n) => !ids.has(n.id)).map((n) => (n.type === "repeat" ? { ...n, children: removeIds(n.children, ids) } : n));
}

export function idsAt(nodes: readonly WorkoutNode[], paths: readonly Path[]): string[] {
  return paths.flatMap((p) => {
    const n = nodeAt(nodes, p);
    return n ? [n.id] : [];
  });
}

/** Selected ids as a contiguous sibling run: parent path + [from, to] indices; null if not siblings. */
export function siblingRun(nodes: readonly WorkoutNode[], ids: readonly string[]): { parent: Path; from: number; to: number } | null {
  const paths = ids.map((id) => pathOf(nodes, id)).filter((p): p is Path => p !== null);
  const first = paths[0];
  if (!first) return null;
  const parent = first.slice(0, -1);
  const key = parent.join(".");
  if (!paths.every((p) => p.slice(0, -1).join(".") === key)) return null;
  const idx = paths.map((p) => p[p.length - 1] ?? 0).sort((a, b) => a - b);
  const from = idx[0] ?? 0;
  const to = idx[idx.length - 1] ?? 0;
  return { parent, from, to };
}

/** Nodes in a sibling run (inclusive). */
export function runNodes(nodes: readonly WorkoutNode[], run: { parent: Path; from: number; to: number }): WorkoutNode[] {
  const list = run.parent.length ? nodeAt(nodes, run.parent) : null;
  const children = run.parent.length ? (list?.type === "repeat" ? list.children : []) : nodes;
  return children.slice(run.from, run.to + 1);
}

/** Replace a sibling run with other nodes. */
export function replaceRun(nodes: readonly WorkoutNode[], run: { parent: Path; from: number; to: number }, repl: WorkoutNode[]): WorkoutNode[] {
  return withChildren(nodes, run.parent, (ch) => [...ch.slice(0, run.from), ...repl, ...ch.slice(run.to + 1)]);
}

export function wrap(nodes: readonly WorkoutNode[], ids: readonly string[], newId: string): WorkoutNode[] | null {
  const run = siblingRun(nodes, ids);
  if (!run) return null;
  const rep: RepeatNode = { type: "repeat", id: newId, count: 2, children: runNodes(nodes, run) };
  return replaceRun(nodes, run, [rep]);
}

export function unwrap(nodes: readonly WorkoutNode[], id: string): WorkoutNode[] | null {
  const p = pathOf(nodes, id);
  const n = p ? nodeAt(nodes, p) : undefined;
  if (!p || n?.type !== "repeat") return null;
  const i = p[p.length - 1] ?? 0;
  return replaceRun(nodes, { parent: p.slice(0, -1), from: i, to: i }, n.children);
}

export function duplicate(nodes: readonly WorkoutNode[], ids: readonly string[]): { nodes: WorkoutNode[]; ids: string[] } | null {
  const run = siblingRun(nodes, ids);
  if (!run) return null;
  const copies = reId(runNodes(nodes, run));
  const out = withChildren(nodes, run.parent, (ch) => [...ch.slice(0, run.to + 1), ...copies, ...ch.slice(run.to + 1)]);
  return { nodes: out, ids: copies.map((c) => c.id) };
}

/** Insert nodes after the given id (or at the end of the root when null). */
export function insertAfter(nodes: readonly WorkoutNode[], afterId: string | null, ins: WorkoutNode[]): WorkoutNode[] {
  const p = afterId ? pathOf(nodes, afterId) : null;
  if (!p) return [...nodes, ...ins];
  const i = p[p.length - 1] ?? 0;
  return withChildren(nodes, p.slice(0, -1), (ch) => [...ch.slice(0, i + 1), ...ins, ...ch.slice(i + 1)]);
}

export type DropPos = "before" | "after" | "into";

/** Move nodes (by id) relative to a target node. Refuses moving a repeat into itself. */
export function move(nodes: readonly WorkoutNode[], ids: readonly string[], targetId: string, pos: DropPos): WorkoutNode[] | null {
  if (ids.includes(targetId)) return null;
  const moving = ids.map((id) => pathOf(nodes, id)).filter((p): p is Path => p !== null).sort(cmpPath).map((p) => nodeAt(nodes, p)).filter((n): n is WorkoutNode => !!n);
  // target must not live inside a moved repeat
  for (const m of moving) if (m.type === "repeat" && pathOf(m.children, targetId)) return null;
  const rest = removeIds(nodes, new Set(ids));
  const tp = pathOf(rest, targetId);
  if (!tp) return null;
  const target = nodeAt(rest, tp);
  if (pos === "into" && target?.type === "repeat") return withChildren(rest, tp, (ch) => [...ch, ...moving]);
  const i = (tp[tp.length - 1] ?? 0) + (pos === "after" ? 1 : 0);
  return withChildren(rest, tp.slice(0, -1), (ch) => [...ch.slice(0, i), ...moving, ...ch.slice(i)]);
}

function cmpPath(a: Path, b: Path): number {
  for (let i = 0; i < Math.min(a.length, b.length); i++) {
    const d = (a[i] ?? 0) - (b[i] ?? 0);
    if (d) return d;
  }
  return a.length - b.length;
}

/** Map a selection across a re-parse (fresh ids): old ids → paths → new ids. */
export function remapSelection(oldNodes: readonly WorkoutNode[], newNodes: readonly WorkoutNode[], ids: readonly string[]): string[] {
  return idsAt(newNodes, ids.map((id) => pathOf(oldNodes, id)).filter((p): p is Path => p !== null));
}

/** Depth-first list of all ids (for diff highlighting). */
export function allIds(nodes: readonly WorkoutNode[]): string[] {
  return nodes.flatMap((n) => (n.type === "repeat" ? [n.id, ...allIds(n.children)] : [n.id]));
}

export function stepWithoutLabel(s: StepNode): StepNode {
  const rest = { ...s };
  delete rest.label;
  return rest;
}
