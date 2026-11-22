// The workout editor (DESIGN §5.7, §16.3 as amended for direction A; #111, #110): a state of the Workouts place, opened
// by New workout, by Edit on one of the rider's workouts, or by "Duplicate & edit" on any (a built-in is read-only, so
// it is always a copy). Three views of one model (./model.ts): the profile, the outline and the text, plus the compiled
// summary. Every edit goes through the model; the core parses (`parse_dsl`) and compiles (`compile_workout`). Undo and
// redo span all three views, by button or Cmd/Ctrl+Z and Shift+Cmd/Ctrl+Z while focus is inside the editor (§5.9's one
// exception). Save stores the workout in the library (`workout_save`), Save as new stores a second one; Ride this rides
// the stored workout by its id (#643: no draft riding), so unsaved changes are asked about first, as leaving is.
import { ArrowLeft, CopyPlus, Play, Redo2, Save, Undo2 } from "lucide-react";
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import { api } from "../api";
import {
  commands,
  type ApiError,
  type CompiledView,
  type PowerTarget,
  type StepEnd,
  type WorkoutNode,
} from "../bindings";
import { setLeaveGuard } from "../router";
import { Dialog } from "../shell/Dialog";
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
  type Saved,
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

/** What the editor opens (DESIGN §5.7): one of the rider's workouts to edit, or a draft that Save stores as new. */
export interface EditorStart {
  /** The rider's stored workout this edits; null for a copy or a new workout. */
  id: string | null;
  name: string;
  /** Reference FTP, W. */
  ftp: number | null;
  nodes: WorkoutNode[];
  /** The canonical text of `nodes`. */
  dsl: string;
  /** The compiled view of `nodes`; null for a new workout, which the editor compiles on opening. */
  compiled: CompiledView | null;
}

/** The library's version of what is open: what Save compares with, and what Ride this rides. */
interface Stored {
  /** Its id; null until the first Save. */
  id: string | null;
  name: string;
  saved: Saved;
}

/** An open question about unsaved changes: before leaving the editor (`then` is the rail's way out) or riding. */
type Ask = { why: "leave"; then?: () => void } | { why: "ride" };

export function WorkoutEditor({
  start,
  onBack,
  onRide,
  loading,
  refusal,
}: {
  start: EditorStart;
  /** Back to the library, with the id of the workout last saved here (to pick it), else null. */
  onBack: (saved: string | null) => void;
  /** Ride the stored workout (`ride_load` by its id, through the app-scope load as the library's Ride this). */
  onRide: (id: string, name: string) => void;
  /** A ride load is in flight (this window's or the core's): Ride this waits. */
  loading: { any: boolean; workoutId: string | null };
  /** The last refused load, shown here when it was for this workout. */
  refusal: { workoutId: string | null; error: ApiError } | null;
}) {
  const [opened] = useState<EditorState>(() =>
    load(
      {
        id: start.id ?? "draft",
        name: start.name,
        ftp: start.ftp,
        nodes: start.nodes,
      },
      start.dsl,
    ),
  );
  const [s, setS] = useState<EditorState>(opened);
  const [name, setName] = useState(start.name);
  const [stored, setStored] = useState<Stored>({
    id: start.id,
    name: start.name,
    saved: opened.saved,
  });
  const [compiled, setCompiled] = useState<CompiledView | null>(start.compiled);
  // A command that failed: the compile of the saved tree, the last parse, the last save. Each clears on its own next
  // success.
  const [compileFailure, setCompileFailure] = useState<ApiError | null>(null);
  const [parseFailure, setParseFailure] = useState<ApiError | null>(null);
  const [saveFailure, setSaveFailure] = useState<ApiError | null>(null);
  const [saving, setSaving] = useState(false);
  const [ask, setAsk] = useState<Ask | null>(null);
  const failure = saveFailure ?? parseFailure ?? compileFailure;
  const root = useRef<HTMLDivElement>(null);

  // The summary and the profile follow the saved tree: each change is compiled by the core; an answer for a tree that
  // is no longer saved is dropped. A copy's first tree is the library's, already compiled; a new workout's is not.
  const saved = s.saved;
  const meta = s.workout;
  const compiledFor = useRef<Saved | null>(start.compiled ? saved : null);
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

  // Unsaved: the name, the tree or a text typed past it differs from the library's version (undoing back to it is
  // clean again). A copy or a new workout not yet saved is not "unsaved" until it is changed: leaving it drops nothing
  // the rider made.
  const dirty =
    name.trim() !== stored.name ||
    s.saved !== stored.saved ||
    s.draft !== s.saved.text;
  const error = compiled?.issues.find((i) => i.severity === "error") ?? null;
  // Why Save cannot store this now; the core refuses the same (library.rs `Library::save`), the page says so first.
  const unsaveable = s.problem
    ? "Fix the text to save the workout."
    : name.trim() === ""
      ? "Give the workout a name to save it."
      : compileFailure
        ? "A workout that does not compile cannot be saved."
        : error
          ? `Can’t save until the error is fixed: ${error.message}`
          : null;
  const canSave =
    unsaveable === null && !saving && (stored.id === null || dirty);

  /** Store what is open, as an update of the stored workout or as a new one; the stored id, or null if refused. */
  const save = async (asNew: boolean): Promise<string | null> => {
    const tree = s.saved;
    const trimmed = name.trim();
    setSaving(true);
    const r = await api(
      commands.workoutSave(asNew ? null : stored.id, {
        name: trimmed,
        ftp: s.workout.ftp,
        nodes: [...tree.nodes],
      }),
    );
    setSaving(false);
    if (r.status !== "ok") {
      setSaveFailure(r.error);
      return null;
    }
    setSaveFailure(null);
    setName(trimmed);
    setStored({ id: r.data, name: trimmed, saved: tree });
    return r.data;
  };

  // Leaving through the rail asks as Back to library does (router.ts `setLeaveGuard`); without unsaved changes it
  // closes the editor and goes.
  const leaveWith = useCallback(
    (then?: () => void) => {
      if (dirty) setAsk({ why: "leave", then });
      else {
        onBack(stored.id);
        then?.();
      }
    },
    [dirty, onBack, stored.id],
  );
  useEffect(
    () =>
      setLeaveGuard((go) => {
        leaveWith(go);
        return true;
      }),
    [leaveWith],
  );

  const rideThis = () => {
    if (stored.id === null) return;
    if (dirty) setAsk({ why: "ride" });
    else onRide(stored.id, stored.name);
  };
  /** The dialog's answers: Save then go on, Discard and go on with the library's version, Cancel. */
  const answer = async (choice: "save" | "discard") => {
    const q = ask;
    if (!q) return;
    const id = choice === "save" ? await save(false) : stored.id;
    if (choice === "save" && id === null) return setAsk(null);
    setAsk(null);
    if (q.why === "ride") {
      if (id !== null)
        onRide(id, choice === "save" ? name.trim() : stored.name);
    } else {
      onBack(id);
      q.then?.();
    }
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
  const busy = loading.workoutId !== null && loading.workoutId === stored.id;
  const shown = name.trim() === "" ? "Untitled workout" : name;
  return (
    <div ref={root} className="page workouts editor">
      <div className="editor-head">
        <div className="editor-title">
          <h1>{shown}</h1>
          <span className="workouts-chip">
            {stored.id === null
              ? "Not saved"
              : dirty
                ? "Unsaved changes"
                : "Saved"}
          </span>
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
          <button type="button" onClick={() => leaveWith()}>
            <ArrowLeft {...icon} />
            Back to library
          </button>
        </div>
      </div>
      <div className="editor-store">
        <label className="editor-name">
          <span>Name</span>
          <input
            type="text"
            value={name}
            maxLength={120}
            spellCheck={false}
            aria-invalid={name.trim() === "" || undefined}
            onChange={(e) => setName(e.target.value)}
          />
        </label>
        <div className="editor-actions">
          <button
            type="button"
            className="primary"
            disabled={!canSave}
            aria-busy={saving || undefined}
            aria-describedby={unsaveable ? "editor-unsaveable" : undefined}
            onClick={() => void save(false)}
          >
            <Save {...icon} />
            {saving ? "Saving…" : "Save"}
          </button>
          {stored.id !== null && (
            <button
              type="button"
              disabled={unsaveable !== null || saving}
              onClick={() => void save(true)}
            >
              <CopyPlus {...icon} />
              Save as new
            </button>
          )}
          <button
            type="button"
            disabled={stored.id === null || loading.any}
            aria-busy={busy || undefined}
            aria-describedby={
              stored.id === null ? "editor-unridden" : undefined
            }
            onClick={rideThis}
          >
            <Play {...icon} />
            {busy ? "Loading ride…" : "Ride this"}
          </button>
        </div>
        {unsaveable && (
          <p id="editor-unsaveable" className="workouts-blocked">
            {unsaveable}
          </p>
        )}
        {stored.id === null && (
          <p id="editor-unridden" className="workouts-blocked">
            Save the workout to ride it.
          </p>
        )}
      </div>
      {refusal && refusal.workoutId === stored.id && (
        <ErrorLine error={refusal.error} />
      )}
      {failure && <ErrorLine error={failure} />}
      <div className="editor-profile">
        {compiled && (
          <Profile
            blocks={compiled.blocks}
            totalS={compiled.totalS}
            marked={marked}
          />
        )}
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
          {compiled ? (
            <Summary compiled={compiled} />
          ) : (
            <p className="workouts-hint">Compiling…</p>
          )}
        </section>
      </div>
      {ask && (
        <Dialog
          title={
            ask.why === "ride"
              ? `Save changes to ${shown} before riding?`
              : `Save changes to ${shown}?`
          }
          answers={[
            {
              label: "Save",
              kind: "primary",
              disabled: unsaveable !== null || saving,
              reason: unsaveable ?? undefined,
              onClick: () => void answer("save"),
            },
            // Ride this asks only once there is a saved version (it is disabled before), which Discard rides.
            { label: "Discard", onClick: () => void answer("discard") },
            { label: "Cancel", onClick: () => setAsk(null) },
          ]}
          focus="Save"
          onCancel={() => setAsk(null)}
        >
          <p>
            {ask.why === "ride"
              ? "Ride this rides the saved workout. Discard rides it as last saved and drops your changes."
              : stored.id === null
                ? "This workout has not been saved. Discard drops it."
                : "Discard drops your changes since the last save."}
          </p>
        </Dialog>
      )}
    </div>
  );
}
