// Path-addressed operations on a WorkoutNode tree. Paths are child-index arrays (like
// FlatBlock.path). All functions mutate the given array in place — in the editor that array is
// a deep $state proxy, so a mutation re-renders exactly the outline rows / profile it touches.
import { reId, type RepeatNode, type WorkoutNode } from "@neo/shared";

export type Path = number[];

export const pathKey = (p: readonly number[]): string => p.join(".");
export const samePath = (a: readonly number[], b: readonly number[]): boolean => a.length === b.length && a.every((x, i) => x === b[i]);
export const isPrefix = (pre: readonly number[], p: readonly number[]): boolean => pre.length <= p.length && pre.every((x, i) => x === p[i]);

export function childrenAt(nodes: WorkoutNode[], parent: readonly number[]): WorkoutNode[] | null {
  let list = nodes;
  for (const i of parent) {
    const n = list[i];
    if (n?.type !== "repeat") return null;
    list = n.children;
  }
  return list;
}

export function nodeAt(nodes: WorkoutNode[], p: readonly number[]): WorkoutNode | undefined {
  if (p.length === 0) return undefined;
  return childrenAt(nodes, p.slice(0, -1))?.[p.at(-1) ?? -1];
}

/** Remove and return the node at p. */
export function removeAt(nodes: WorkoutNode[], p: readonly number[]): WorkoutNode | undefined {
  const list = childrenAt(nodes, p.slice(0, -1));
  const i = p.at(-1);
  if (!list || i === undefined) return undefined;
  return list.splice(i, 1)[0];
}

export function insertAt(nodes: WorkoutNode[], parent: readonly number[], index: number, ...items: WorkoutNode[]): boolean {
  const list = childrenAt(nodes, parent);
  if (!list) return false;
  list.splice(Math.max(0, Math.min(list.length, index)), 0, ...items);
  return true;
}

/**
 * Wrap the contiguous sibling range [first, last] (paths sharing a parent) into a 2× repeat.
 * Returns the path of the new repeat.
 */
export function wrap(nodes: WorkoutNode[], first: Path, last: Path): Path | null {
  const parent = first.slice(0, -1);
  const a = first.at(-1);
  const b = last.at(-1);
  const list = childrenAt(nodes, parent);
  if (!list || a === undefined || b === undefined || !samePath(parent, last.slice(0, -1))) return null;
  const lo = Math.min(a, b);
  const hi = Math.max(a, b);
  const items = list.splice(lo, hi - lo + 1);
  const rep: RepeatNode = { type: "repeat", id: `r-${Math.random().toString(36).slice(2, 9)}`, count: 2, children: items };
  list.splice(lo, 0, rep);
  return [...parent, lo];
}

/** Replace a repeat by its children. Returns the path of the first child. */
export function unwrap(nodes: WorkoutNode[], p: Path): Path | null {
  const n = nodeAt(nodes, p);
  const list = childrenAt(nodes, p.slice(0, -1));
  const i = p.at(-1);
  if (n?.type !== "repeat" || !list || i === undefined) return null;
  list.splice(i, 1, ...n.children);
  return p;
}

export function duplicate(nodes: WorkoutNode[], p: Path): Path | null {
  const n = nodeAt(nodes, p);
  const i = p.at(-1);
  if (!n || i === undefined) return null;
  // n may be a reactive proxy: clone through JSON (structuredClone rejects proxies)
  insertAt(nodes, p.slice(0, -1), i + 1, ...reId([JSON.parse(JSON.stringify(n)) as WorkoutNode]));
  return [...p.slice(0, -1), i + 1];
}

/**
 * Move node `from` to become child `index` of `toParent`. Handles index shifts when moving
 * within the same list and refuses to move a repeat into itself. Returns the new path.
 */
export function move(nodes: WorkoutNode[], from: Path, toParent: Path, index: number): Path | null {
  if (isPrefix(from, toParent)) return null; // into itself / own descendant
  const fromParent = from.slice(0, -1);
  const fi = from.at(-1);
  if (fi === undefined) return null;
  // Removing `from` shifts later siblings (and any path through them) left by one.
  const adjust = (p: Path): Path => {
    if (p.length > fromParent.length && isPrefix(fromParent, p) && (p[fromParent.length] ?? 0) > fi) {
      const q = [...p];
      q[fromParent.length] = (q[fromParent.length] ?? 0) - 1;
      return q;
    }
    return p;
  };
  let target = adjust(toParent);
  let at = index;
  if (samePath(fromParent, toParent) && index > fi) at -= 1;
  const n = removeAt(nodes, from);
  if (!n) return null;
  if (!insertAt(nodes, target, at, n)) {
    insertAt(nodes, fromParent, fi, n); // restore
    return null;
  }
  target = [...target, Math.max(0, Math.min(at, (childrenAt(nodes, target)?.length ?? 1) - 1))];
  return target;
}

/** Depth-first list of all node paths (outline order). */
export function allPaths(nodes: readonly WorkoutNode[], parent: Path = []): Path[] {
  const out: Path[] = [];
  nodes.forEach((n, i) => {
    const p = [...parent, i];
    out.push(p);
    if (n.type === "repeat") out.push(...allPaths(n.children, p));
  });
  return out;
}
