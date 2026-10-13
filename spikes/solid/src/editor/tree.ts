// Pure, immutable operations on the workout tree. All editor views go through these, so
// every edit is one new tree → one undo step, and the store reconciles it by id.
import type { RepeatNode, WorkoutNode } from "@neo/shared";
import { reId } from "@neo/shared";

export type Path = number[];

export function pathOf(nodes: readonly WorkoutNode[], id: string, base: Path = []): Path | null {
  for (const [i, n] of nodes.entries()) {
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
  let n: WorkoutNode | undefined;
  for (const i of path) {
    n = list[i];
    if (!n) return undefined;
    list = n.type === "repeat" ? n.children : [];
  }
  return n;
}

export function findById(nodes: readonly WorkoutNode[], id: string): WorkoutNode | undefined {
  const p = pathOf(nodes, id);
  return p ? nodeAt(nodes, p) : undefined;
}

/** Replace the child list at parentPath (root = []). */
function withChildren(nodes: readonly WorkoutNode[], parentPath: Path, fn: (children: WorkoutNode[]) => WorkoutNode[]): WorkoutNode[] {
  if (parentPath.length === 0) return fn([...nodes]);
  const [head, ...rest] = parentPath;
  return nodes.map((n, i) => {
    if (i !== head || n.type !== "repeat") return n;
    return { ...n, children: withChildren(n.children, rest, fn) };
  });
}

export function updateNode(nodes: readonly WorkoutNode[], id: string, fn: (n: WorkoutNode) => WorkoutNode): WorkoutNode[] {
  return nodes.map((n) => {
    if (n.id === id) return fn(n);
    if (n.type === "repeat") return { ...n, children: updateNode(n.children, id, fn) };
    return n;
  });
}

export function removeIds(nodes: readonly WorkoutNode[], ids: ReadonlySet<string>): WorkoutNode[] {
  return nodes.filter((n) => !ids.has(n.id)).map((n) => (n.type === "repeat" ? { ...n, children: removeIds(n.children, ids) } : n));
}

export function insertAt(nodes: readonly WorkoutNode[], parentPath: Path, index: number, items: readonly WorkoutNode[]): WorkoutNode[] {
  return withChildren(nodes, parentPath, (ch) => {
    ch.splice(Math.max(0, Math.min(index, ch.length)), 0, ...items);
    return ch;
  });
}

/** Selected ids that are siblings: their parent path and sorted indices (else null). */
export function siblingRange(nodes: readonly WorkoutNode[], ids: ReadonlySet<string>): { parent: Path; from: number; to: number } | null {
  const paths = [...ids].map((id) => pathOf(nodes, id)).filter((p): p is Path => p !== null);
  if (!paths.length) return null;
  const first = paths[0] ?? [];
  const parent = first.slice(0, -1);
  if (!paths.every((p) => p.length === first.length && p.slice(0, -1).join(".") === parent.join("."))) return null;
  const idx = paths.map((p) => p[p.length - 1] ?? 0);
  return { parent, from: Math.min(...idx), to: Math.max(...idx) };
}

/** Wrap a contiguous sibling range (covering the selection) in a 2× repeat. */
export function wrap(nodes: readonly WorkoutNode[], ids: ReadonlySet<string>, newId: string): WorkoutNode[] | null {
  const r = siblingRange(nodes, ids);
  if (!r) return null;
  return withChildren(nodes, r.parent, (ch) => {
    const taken = ch.slice(r.from, r.to + 1);
    const rep: RepeatNode = { type: "repeat", id: newId, count: 2, children: taken };
    ch.splice(r.from, taken.length, rep);
    return ch;
  });
}

export function unwrap(nodes: readonly WorkoutNode[], id: string): WorkoutNode[] | null {
  const p = pathOf(nodes, id);
  const n = p ? nodeAt(nodes, p) : undefined;
  if (!p || !n || n.type !== "repeat") return null;
  const parent = p.slice(0, -1);
  const i = p[p.length - 1] ?? 0;
  return withChildren(nodes, parent, (ch) => {
    ch.splice(i, 1, ...n.children);
    return ch;
  });
}

/** Duplicate the selected sibling range right after itself (fresh ids). */
export function duplicate(nodes: readonly WorkoutNode[], ids: ReadonlySet<string>): { nodes: WorkoutNode[]; newIds: string[] } | null {
  const r = siblingRange(nodes, ids);
  if (!r) return null;
  let newIds: string[] = [];
  const out = withChildren(nodes, r.parent, (ch) => {
    const copies = reId(ch.slice(r.from, r.to + 1));
    newIds = copies.map((c) => c.id);
    ch.splice(r.to + 1, 0, ...copies);
    return ch;
  });
  return { nodes: out, newIds };
}

/** Replace the selected sibling range with new nodes (AI rewrite / snippet replace). */
export function replaceRange(nodes: readonly WorkoutNode[], ids: ReadonlySet<string>, items: readonly WorkoutNode[]): WorkoutNode[] | null {
  const r = siblingRange(nodes, ids);
  if (!r) return null;
  return withChildren(nodes, r.parent, (ch) => {
    ch.splice(r.from, r.to - r.from + 1, ...items);
    return ch;
  });
}

export function selectedNodes(nodes: readonly WorkoutNode[], ids: ReadonlySet<string>): WorkoutNode[] {
  const r = siblingRange(nodes, ids);
  if (!r) return [];
  const parent = r.parent.length ? nodeAt(nodes, r.parent) : undefined;
  const list = r.parent.length ? (parent?.type === "repeat" ? parent.children : []) : nodes;
  return list.slice(r.from, r.to + 1);
}

function contains(n: WorkoutNode, id: string): boolean {
  return n.type === "repeat" && n.children.some((c) => c.id === id || contains(c, id));
}

/**
 * Move node `id` to (parentPath, index) — the target is expressed relative to the tree
 * *before* removal; we resolve it by anchoring to a sibling id when possible.
 */
export function move(nodes: readonly WorkoutNode[], id: string, target: { anchorId: string; where: "before" | "after" | "into" }): WorkoutNode[] | null {
  if (id === target.anchorId) return null;
  const node = findById(nodes, id);
  if (!node || contains(node, target.anchorId)) return null; // can't drop into own subtree
  const without = removeIds(nodes, new Set([id]));
  const ap = pathOf(without, target.anchorId);
  if (!ap) return null;
  if (target.where === "into") {
    const anchor = nodeAt(without, ap);
    if (anchor?.type !== "repeat") return null;
    return insertAt(without, ap, anchor.children.length, [node]);
  }
  const parent = ap.slice(0, -1);
  const i = (ap[ap.length - 1] ?? 0) + (target.where === "after" ? 1 : 0);
  return insertAt(without, parent, i, [node]);
}

/**
 * Text → tree: parseDsl assigns fresh ids, so carry the previous ids over by tree position
 * when the node type matches. Keeps outline rows (and the selection) stable while typing.
 */
export function carryIds(prev: readonly WorkoutNode[], next: readonly WorkoutNode[]): WorkoutNode[] {
  return next.map((n, i) => {
    const p = prev[i];
    if (!p || p.type !== n.type) return n;
    if (n.type === "repeat" && p.type === "repeat") return { ...n, id: p.id, children: carryIds(p.children, n.children) };
    return { ...n, id: p.id };
  });
}

export function allIds(nodes: readonly WorkoutNode[], out: Set<string> = new Set()): Set<string> {
  for (const n of nodes) {
    out.add(n.id);
    if (n.type === "repeat") allIds(n.children, out);
  }
  return out;
}
