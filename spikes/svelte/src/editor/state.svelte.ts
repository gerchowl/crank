// Workout editor state. One deep $state tree is the single source of truth; the profile,
// outline and DSL views all read and mutate it directly.
//
// Undo/redo needs no command objects: an effect serialises the tree (deep-reading it, so it
// tracks every field) and records a history entry whenever it changes. Effects are batched,
// so one handler doing several mutations becomes one entry; rapid edits (drag, typing) within
// COALESCE_MS are merged into one.
import { untrack } from "svelte";
import { SvelteSet } from "svelte/reactivity";
import { reId, type CompiledWorkout, type DslError, type RewriteIntent, type Workout, type WorkoutNode } from "@neo/shared";
import { backend } from "../lib/backend";
import { childrenAt, duplicate, move, nodeAt, removeAt, samePath, unwrap, wrap, type Path } from "./tree";

const COALESCE_MS = 600;
const SNIPPET_KEY = "neo-trainer.snippets.v1";

export interface Proposal {
  parent: Path;
  lo: number;
  oldCount: number;
  nodes: WorkoutNode[];
  intent: RewriteIntent;
}

function loadSnippets(): { name: string; nodes: WorkoutNode[] }[] {
  try {
    const raw = localStorage.getItem(SNIPPET_KEY);
    return raw ? (JSON.parse(raw) as { name: string; nodes: WorkoutNode[] }[]) : [];
  } catch {
    return [];
  }
}

function initialWorkout(): Workout {
  const cur = backend.ride.current();
  if (cur) return structuredClone(cur.workout);
  return { id: "new", name: "New workout", ftp: backend.athlete.ftp, nodes: [] };
}

export class EditorState {
  workout = $state<Workout>(initialWorkout());
  /** Selection = a contiguous sibling range from anchor to focus (same parent). */
  anchor = $state<Path | null>(null);
  focus = $state<Path | null>(null);
  collapsed = new SvelteSet<string>();

  /** DSL text being typed (null = show the canonical print of the tree). */
  draft = $state<string | null>(null);
  dslError = $state<DslError | null>(null);

  // $state.raw: immutable values replaced wholesale. (A deep $state proxy can't go through
  // structuredClone — a runtime error the type checker doesn't see.)
  proposal = $state.raw<Proposal | null>(null);
  aiBusy = $state<RewriteIntent | null>(null);
  userSnippets = $state.raw(loadSnippets());

  canUndo = $state(false);
  canRedo = $state(false);
  #past: string[] = [];
  #future: string[] = [];
  #current = "";
  #lastAt = 0;
  #restoring = false;

  compiled = $derived<CompiledWorkout>(backend.compileWorkout($state.snapshot(this.workout)));
  printed = $derived(backend.printDsl($state.snapshot(this.workout.nodes)));
  text = $derived(this.draft ?? this.printed);

  selection = $derived.by((): Path[] => {
    const a = this.anchor;
    const f = this.focus;
    if (!f) return [];
    if (!a || !samePath(a.slice(0, -1), f.slice(0, -1))) return [f];
    const parent = f.slice(0, -1);
    const lo = Math.min(a.at(-1) ?? 0, f.at(-1) ?? 0);
    const hi = Math.max(a.at(-1) ?? 0, f.at(-1) ?? 0);
    return Array.from({ length: hi - lo + 1 }, (_, k) => [...parent, lo + k]);
  });

  /** Tree with the AI proposal spliced in (P24 "new"), or null. */
  previewNodes = $derived.by((): WorkoutNode[] | null => {
    const p = this.proposal;
    if (!p) return null;
    const nodes = $state.snapshot(this.workout.nodes);
    childrenAt(nodes, p.parent)?.splice(p.lo, p.oldCount, ...structuredClone(p.nodes));
    return nodes;
  });
  previewCompiled = $derived(this.previewNodes ? backend.compileWorkout({ ...$state.snapshot(this.workout), nodes: this.previewNodes }) : null);

  snippets = $derived([...backend.snippets(), ...this.userSnippets]);

  constructor() {
    $effect.root(() => {
      $effect(() => {
        const s = JSON.stringify(this.workout);
        untrack(() => {
          this.#record(s);
        });
      });
    });
  }

  #record(s: string): void {
    if (s === this.#current) return;
    if (this.#restoring || this.#current === "") {
      this.#restoring = false;
      this.#current = s;
      return;
    }
    const now = performance.now();
    if (now - this.#lastAt > COALESCE_MS) this.#past.push(this.#current);
    this.#lastAt = now;
    this.#current = s;
    this.#future = [];
    this.canUndo = this.#past.length > 0;
    this.canRedo = false;
  }

  #restore(s: string): void {
    this.#restoring = true;
    this.#current = s;
    this.workout = JSON.parse(s) as Workout;
    this.draft = null;
    this.dslError = null;
    this.proposal = null;
    this.#lastAt = 0;
    this.canUndo = this.#past.length > 0;
    this.canRedo = this.#future.length > 0;
    // keep the selection if it still resolves
    if (this.focus && !nodeAt(this.workout.nodes, this.focus)) {
      this.focus = null;
      this.anchor = null;
    }
  }

  undo(): void {
    const prev = this.#past.pop();
    if (prev === undefined) return;
    this.#future.push(this.#current);
    this.#restore(prev);
  }

  redo(): void {
    const next = this.#future.pop();
    if (next === undefined) return;
    this.#past.push(this.#current);
    this.#restore(next);
  }

  /** Break coalescing so the next change is its own undo step (discrete commands). */
  checkpoint(): void {
    this.#lastAt = 0;
  }

  load(w: Workout): void {
    this.checkpoint();
    this.workout = structuredClone(w);
    this.anchor = this.focus = null;
    this.draft = null;
    this.dslError = null;
    this.proposal = null;
  }

  select(p: Path | null, extend = false): void {
    if (extend && this.focus && p && samePath(this.focus.slice(0, -1), p.slice(0, -1))) {
      this.anchor ??= this.focus;
      this.focus = p;
      return;
    }
    this.anchor = p;
    this.focus = p;
  }

  isSelected(p: Path): boolean {
    return this.selection.some((s) => samePath(s, p));
  }

  // ---------- structural commands (outline buttons + keyboard) ----------

  wrapSelection(): void {
    const sel = this.selection;
    const first = sel[0];
    const last = sel.at(-1);
    if (!first || !last) return;
    this.checkpoint();
    const p = wrap(this.workout.nodes, first, last);
    if (p) this.select(p);
  }

  unwrapFocus(): void {
    const f = this.focus;
    if (!f || nodeAt(this.workout.nodes, f)?.type !== "repeat") return;
    this.checkpoint();
    const p = unwrap(this.workout.nodes, f);
    if (p) this.select(p);
  }

  duplicateFocus(): void {
    const f = this.focus;
    if (!f) return;
    this.checkpoint();
    const p = duplicate(this.workout.nodes, f);
    if (p) this.select(p);
  }

  deleteSelection(): void {
    const sel = this.selection;
    if (!sel.length) return;
    this.checkpoint();
    // delete from the end so earlier paths stay valid
    for (const p of [...sel].reverse()) removeAt(this.workout.nodes, p);
    const first = sel[0];
    if (!first) return;
    const siblings = childrenAt(this.workout.nodes, first.slice(0, -1));
    const i = Math.min(first.at(-1) ?? 0, (siblings?.length ?? 0) - 1);
    this.select(i >= 0 ? [...first.slice(0, -1), i] : first.length > 1 ? first.slice(0, -1) : null);
  }

  moveFocus(delta: -1 | 1): void {
    const f = this.focus;
    if (!f) return;
    const parent = f.slice(0, -1);
    const i = f.at(-1) ?? 0;
    const n = childrenAt(this.workout.nodes, parent)?.length ?? 0;
    const j = i + delta;
    if (j < 0 || j >= n) return;
    this.checkpoint();
    const p = move(this.workout.nodes, f, parent, delta > 0 ? j + 1 : j);
    if (p) this.select(p);
  }

  /** Drag-and-drop from the outline: drop `from` before/after/into `target`. */
  dropOn(from: Path, target: Path, where: "before" | "after" | "into"): void {
    this.checkpoint();
    const toParent = where === "into" ? target : target.slice(0, -1);
    const index = where === "into" ? 0 : (target.at(-1) ?? 0) + (where === "after" ? 1 : 0);
    const p = move(this.workout.nodes, from, toParent, index);
    if (p) this.select(p);
  }

  /** Arrow-key navigation through visible outline rows. */
  step(delta: -1 | 1, visible: Path[]): void {
    const f = this.focus;
    const i = f ? visible.findIndex((p) => samePath(p, f)) : -1;
    const next = visible[Math.max(0, Math.min(visible.length - 1, i + delta))];
    if (next) this.select(next);
  }

  // ---------- DSL ----------

  editText(text: string): void {
    this.draft = text;
    const r = backend.parseDsl(text, this.workout.ftp);
    if ("error" in r) {
      this.dslError = r.error;
      return;
    }
    this.dslError = null;
    // Fresh ids from every parse: selection is by path, so it survives.
    this.workout.nodes = r.nodes;
    if (this.focus && !nodeAt(this.workout.nodes, this.focus)) this.select(null);
  }

  commitText(): void {
    if (!this.dslError) this.draft = null;
  }

  // ---------- AI rewrite (P24) ----------

  async rewrite(intent: RewriteIntent): Promise<void> {
    const sel = this.selection;
    const first = sel[0];
    if (!first) return;
    const parent = first.slice(0, -1);
    const lo = first.at(-1) ?? 0;
    const old = sel.map((p) => nodeAt(this.workout.nodes, p)).filter((n): n is WorkoutNode => n !== undefined);
    this.aiBusy = intent;
    try {
      const nodes = await backend.planner.rewrite($state.snapshot(old), intent);
      this.proposal = { parent, lo, oldCount: old.length, nodes, intent };
    } finally {
      this.aiBusy = null;
    }
  }

  acceptProposal(): void {
    const p = this.proposal;
    if (!p) return;
    this.checkpoint();
    childrenAt(this.workout.nodes, p.parent)?.splice(p.lo, p.oldCount, ...reId(p.nodes));
    this.anchor = [...p.parent, p.lo];
    this.focus = [...p.parent, p.lo + p.nodes.length - 1];
    this.proposal = null;
  }

  rejectProposal(): void {
    this.proposal = null;
  }

  // ---------- snippets ----------

  insertSnippet(nodes: WorkoutNode[]): void {
    this.checkpoint();
    const f = this.focus;
    const parent = f ? f.slice(0, -1) : [];
    const at = f ? (f.at(-1) ?? -1) + 1 : this.workout.nodes.length;
    childrenAt(this.workout.nodes, parent)?.splice(at, 0, ...reId(nodes));
    this.select([...parent, at]);
  }

  saveSnippet(name: string): void {
    const nodes = this.selection.map((p) => nodeAt(this.workout.nodes, p)).filter((n): n is WorkoutNode => n !== undefined);
    if (!nodes.length) return;
    this.userSnippets = [...this.userSnippets, { name, nodes: reId($state.snapshot(nodes)) }];
    localStorage.setItem(SNIPPET_KEY, JSON.stringify(this.userSnippets));
  }
}

export const editor = new EditorState();
