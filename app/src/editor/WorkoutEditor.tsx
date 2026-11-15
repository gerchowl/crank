// The workout editor (DESIGN §5.7, §16.3 as amended for direction A; #111): a state of the Workouts place, opened by
// "Duplicate & edit" on a library workout, so the library stays read-only and the editor works on a copy. Three views
// of one model (./model.ts): the profile, the outline and the text, plus the compiled summary. Every edit goes through
// the model; the core parses (`parse_dsl`) and compiles (`compile_workout`). Undo and redo span all three views, by
// button or Cmd/Ctrl+Z and Shift+Cmd/Ctrl+Z while focus is inside the editor (§5.9's one exception). The draft lives in
// memory: saving is the library's (#110); riding a draft is #643.
import { ArrowLeft, Redo2, Undo2 } from "lucide-react";
import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { api } from "../api";
import {
  commands,
  type ApiError,
  type CompiledView,
  type PowerTarget,
  type StepEnd,
  type WorkoutDetail,
  type WorkoutNode,
} from "../bindings";
import { ErrorLine, Profile, Summary, formatDuration } from "../views/Workouts";
import {
  canRedo,
  canUndo,
  load,
  parsed,
  redo,
  select,
  typed,
  undo,
  type EditorState,
} from "./model";
import "./editor.css";

const icon = { size: 16, strokeWidth: 1.5, "aria-hidden": true } as const;

/** The clock keystrokes are timed by for undo groups (model.ts `GROUP_MS`); tests set it to step past or within one. */
export const typingClock = { now: () => performance.now() };
const num = (n: number | null) => (n === null ? "—" : String(n));

function endText(e: StepEnd): string {
  switch (e.kind) {
    case "time":
      return formatDuration(e.s);
    case "kj":
      return `${num(e.kj)} kJ`;
    case "lap":
      return "lap";
    case "hr_recovery":
      return `HR < ${num(e.targetBpm)} bpm, ${formatDuration(e.minS)}–${formatDuration(e.maxS)}`;
  }
}
function targetText(t: PowerTarget): string {
  switch (t.kind) {
    case "pct_ftp":
      return `${num(t.value)} %`;
    case "watts":
      return `${num(t.value)} W`;
    case "range":
      return `${num(t.lo)}–${num(t.hi)} %`;
    case "ramp":
      return `ramp ${num(t.from)}–${num(t.to)} %`;
    case "hr":
      return `${num(t.bpm)} bpm`;
    case "free":
      return "free";
  }
}
/** One outline row's words: a step's label, length and target; a repeat's count and label. */
export function describe(n: WorkoutNode): string {
  if (n.type === "repeat")
    return [`${num(n.count)} ×`, n.label].filter(Boolean).join(" ");
  return [n.label, endText(n.end), targetText(n.target)]
    .filter(Boolean)
    .join(" · ");
}

/** The tree in reading order, each node with its depth (0 = top level). */
function flatten(
  nodes: readonly WorkoutNode[],
  depth = 0,
): { node: WorkoutNode; depth: number }[] {
  return nodes.flatMap((node) => [
    { node, depth },
    ...(node.type === "repeat" ? flatten(node.children, depth + 1) : []),
  ]);
}

/**
 * The outline: the tree as one single-select listbox (WAI-ARIA APG, selection follows focus, as the library), a row
 * per node indented by its depth; a row is as tall as its words need, so a long label wraps and is never cut.
 */
function Outline({
  nodes,
  selection,
  onSelect,
}: {
  nodes: readonly WorkoutNode[];
  selection: string | null;
  onSelect: (id: string) => void;
}) {
  const rows = flatten(nodes);
  const refs = useRef(new Map<string, HTMLLIElement>());
  const stop = rows.some((r) => r.node.id === selection)
    ? selection
    : (rows[0]?.node.id ?? null);
  const onKey = (e: KeyboardEvent<HTMLUListElement>) => {
    const at = rows.findIndex((r) => r.node.id === stop);
    const to =
      e.key === "ArrowDown"
        ? Math.min(at + 1, rows.length - 1)
        : e.key === "ArrowUp"
          ? Math.max(at - 1, 0)
          : e.key === "Home"
            ? 0
            : e.key === "End"
              ? rows.length - 1
              : null;
    if (to === null || !rows[to]) return;
    e.preventDefault();
    const id = rows[to].node.id;
    refs.current.get(id)?.focus();
    onSelect(id);
  };
  return (
    <ul
      role="listbox"
      aria-label="Outline"
      className="editor-outline"
      onKeyDown={onKey}
    >
      {rows.map(({ node, depth }) => (
        <li
          key={node.id}
          ref={(el) => {
            if (el) refs.current.set(node.id, el);
            else refs.current.delete(node.id);
          }}
          role="option"
          aria-selected={selection === node.id}
          tabIndex={stop === node.id ? 0 : -1}
          data-depth={Math.min(depth, 2)}
          onClick={() => onSelect(node.id)}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              onSelect(node.id);
            }
          }}
        >
          {depth > 0 && <span className="sr-only">in the repeat above, </span>}
          {describe(node)}
        </li>
      ))}
    </ul>
  );
}

/** Every node id under (and including) the selected one: the profile marks all their blocks. */
function subtree(
  nodes: readonly WorkoutNode[],
  id: string | null,
): Set<string> {
  const out = new Set<string>();
  const walk = (ns: readonly WorkoutNode[], inside: boolean) => {
    for (const n of ns) {
      const here = inside || n.id === id;
      if (here) out.add(n.id);
      if (n.type === "repeat") walk(n.children, here);
    }
  };
  if (id !== null) walk(nodes, false);
  return out;
}

/** 1-based line and column of a UTF-16 offset, as the core reports it (docs/DSL.md "Errors"). */
export function lineColumn(text: string, offset: number) {
  const before = text.slice(0, offset).split("\n");
  return { line: before.length, column: (before.at(-1)?.length ?? 0) + 1 };
}

/**
 * The problem's line, split around its span (`DslProblem` offset and length, UTF-16) so the view can mark it: a
 * textarea cannot style a range of its own text. A span that runs past its line is cut at the line's end; an empty
 * one (a token missing at the end) marks nothing.
 */
export function problemLine(text: string, offset: number, length: number) {
  const start = text.lastIndexOf("\n", offset - 1) + 1;
  const nl = text.indexOf("\n", offset);
  const end = nl === -1 ? text.length : nl;
  const to = Math.min(offset + length, end);
  return {
    ...lineColumn(text, offset),
    before: text.slice(start, offset),
    span: text.slice(offset, to),
    after: text.slice(to, end),
  };
}

export function WorkoutEditor({
  from,
  onBack,
}: {
  /** The library workout the draft is a copy of. */
  from: WorkoutDetail;
  onBack: () => void;
}) {
  const [s, setS] = useState<EditorState>(() =>
    load(
      {
        ...from.workout,
        id: `copy-of-${from.workout.id}`,
        name: `${from.workout.name} (copy)`,
      },
      from.dsl,
    ),
  );
  const [compiled, setCompiled] = useState<CompiledView>(from.compiled);
  // A command that failed: the compile of the saved tree, or the last parse. Each clears on its own next success.
  const [compileFailure, setCompileFailure] = useState<ApiError | null>(null);
  const [parseFailure, setParseFailure] = useState<ApiError | null>(null);
  const failure = parseFailure ?? compileFailure;
  const root = useRef<HTMLDivElement>(null);

  // The summary and the profile follow the saved tree: each change is compiled by the core; an answer for a tree that
  // is no longer saved is dropped. The copy's first tree is the library's, already compiled.
  const saved = s.saved;
  const meta = s.workout;
  const compiledFor = useRef(saved);
  useEffect(() => {
    if (compiledFor.current === saved) return;
    let current = true;
    void api(
      commands.compileWorkout({ ...meta, nodes: [...saved.nodes] }),
    ).then((r) => {
      if (!current) return;
      compiledFor.current = saved;
      if (r.status === "ok") {
        setCompiled(r.data);
        setCompileFailure(null);
      } else setCompileFailure(r.error);
    });
    return () => {
      current = false;
    };
  }, [saved, meta]);

  const edit = (text: string, at: number) => {
    setS((st) => typed(st, text, at));
    void api(commands.parseDsl(text, s.workout.ftp)).then((r) => {
      if (r.status === "ok") {
        setParseFailure(null);
        setS((st) => parsed(st, text, r.data));
      } else setParseFailure(r.error);
    });
  };

  // Cmd/Ctrl+Z and Shift+Cmd/Ctrl+Z, only while focus is inside the editor (DESIGN §5.9): the text's own undo would
  // reach the text view alone, so the editor takes the keys and undoes in all three.
  useEffect(() => {
    const el = root.current;
    if (!el) return;
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.altKey || e.key.toLowerCase() !== "z")
        return;
      e.preventDefault();
      setS((st) => (e.shiftKey ? redo(st) : undo(st)));
    };
    el.addEventListener("keydown", onKey);
    return () => el.removeEventListener("keydown", onKey);
  }, []);

  const marked = subtree(saved.nodes, s.selection);
  const at = s.problem
    ? problemLine(s.draft, s.problem.offset, s.problem.length)
    : null;
  return (
    <div ref={root} className="page workouts editor">
      <div className="editor-head">
        <div className="editor-title">
          <h1>{s.workout.name}</h1>
          <span className="workouts-chip">Draft, not saved</span>
        </div>
        <div className="editor-actions">
          <button
            type="button"
            disabled={!canUndo(s)}
            onClick={() => setS(undo)}
          >
            <Undo2 {...icon} />
            Undo
          </button>
          <button
            type="button"
            disabled={!canRedo(s)}
            onClick={() => setS(redo)}
          >
            <Redo2 {...icon} />
            Redo
          </button>
          <button type="button" onClick={onBack}>
            <ArrowLeft {...icon} />
            Back to library
          </button>
        </div>
      </div>
      {failure && <ErrorLine error={failure} />}
      <div className="editor-profile">
        <Profile
          blocks={compiled.blocks}
          totalS={compiled.totalS}
          marked={marked}
        />
      </div>
      <div className="editor-panes">
        <section className="editor-pane" aria-labelledby="editor-outline-h">
          <h2 id="editor-outline-h">Outline</h2>
          {saved.nodes.length === 0 ? (
            <p className="workouts-hint">No steps yet: write one in Text.</p>
          ) : (
            <Outline
              nodes={saved.nodes}
              selection={s.selection}
              onSelect={(id) => setS((st) => select(st, id))}
            />
          )}
        </section>
        <section
          className="editor-pane editor-text"
          aria-labelledby="editor-text-h"
        >
          <h2 id="editor-text-h">
            <label htmlFor="editor-dsl">Text</label>
          </h2>
          <textarea
            id="editor-dsl"
            rows={12}
            spellCheck={false}
            value={s.draft}
            aria-invalid={s.problem !== null || undefined}
            aria-describedby={s.problem ? "editor-problem" : undefined}
            onChange={(e) => edit(e.target.value, typingClock.now())}
          />
          {s.problem && at && (
            <p
              id="editor-problem"
              className="editor-problem"
              data-state="error"
            >
              <span data-state-label>⚠</span> {s.problem.message} (line{" "}
              {at.line}, column {at.column}). The saved workout is unchanged.{" "}
              <code className="editor-problem-line">
                {at.before}
                {at.span && <mark>{at.span}</mark>}
                {at.after}
              </code>
            </p>
          )}
        </section>
        <section className="editor-pane" aria-labelledby="editor-summary-h">
          <h2 id="editor-summary-h">Summary</h2>
          <Summary compiled={compiled} />
        </section>
      </div>
    </div>
  );
}
