// Editing the tree from the outline (DESIGN §5.7 "Outline editing", #653): the outline's toolbar (add, duplicate,
// wrap in a repeat, unwrap, move up and down, delete) and the selected node's form (label, duration, power; a repeat's
// label and count). Both only build the next tree; WorkoutEditor commits it through the core (`print_dsl`, then
// `parse_dsl` of that text) as one undo step, or shows the core's refusal. Nothing here parses or prints DSL.
import {
  ArrowDown,
  ArrowUp,
  Copy,
  Plus,
  Repeat2,
  Trash2,
  Ungroup,
} from "lucide-react";
import {
  useId,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import type { PowerTarget, WorkoutNode } from "../bindings";
import {
  afterRemoval,
  copyNode,
  insertAfter,
  moveNode,
  nodeAt,
  pathOf,
  removeNode,
  replaceNode,
  siblingsOf,
  unwrapNode,
  wrapNode,
} from "./model";

const icon = { size: 16, strokeWidth: 1.5, "aria-hidden": true } as const;

/** An outline edit: the next tree, and the node to select (and focus) once it is committed. */
export interface OutlineChange {
  nodes: WorkoutNode[];
  select: string | null;
}

/** The step "Add step" inserts: 5 min at 50 % FTP, an easy block that compiles in any workout. */
export const newStep = (id: string): WorkoutNode => ({
  type: "step",
  id,
  label: null,
  end: { kind: "time", s: 300 },
  target: { kind: "pct_ftp", value: 50 },
  cadence: null,
});
/** A new repeat's count (the rider sets it in the form). */
export const WRAP_COUNT = 2;

/** The toolbar's actions on `selection` in `nodes`, in toolbar order; each null when it does not apply there. */
export function outlineActions(
  nodes: readonly WorkoutNode[],
  selection: string | null,
): {
  label: string;
  icon: ReactNode;
  /** The edit, given a source of fresh ids for the nodes it adds; null when the action does not apply. */
  change: ((fresh: () => string) => OutlineChange) | null;
}[] {
  const node = selection === null ? null : findNode(nodes, selection);
  const at = node === null ? null : siblingsOf(nodes, node.id);
  const on = (f: (n: WorkoutNode, fresh: () => string) => OutlineChange) =>
    node === null ? null : (fresh: () => string) => f(node, fresh);
  return [
    {
      label: "Add step",
      icon: <Plus {...icon} />,
      // After the selected node, in its list; at the end with nothing selected (an empty workout's first step).
      change: (fresh) => {
        const step = newStep(fresh());
        return {
          nodes: insertAfter(nodes, node?.id ?? null, step),
          select: step.id,
        };
      },
    },
    {
      label: "Duplicate",
      icon: <Copy {...icon} />,
      change: on((n, fresh) => {
        const copy = copyNode(n, fresh);
        return { nodes: insertAfter(nodes, n.id, copy), select: copy.id };
      }),
    },
    {
      label: "Wrap in repeat",
      icon: <Repeat2 {...icon} />,
      change: on((n, fresh) => {
        const id = fresh();
        return { nodes: wrapNode(nodes, n.id, WRAP_COUNT, id), select: id };
      }),
    },
    {
      label: "Unwrap",
      icon: <Ungroup {...icon} />,
      change:
        node?.type === "repeat"
          ? () => ({
              nodes: unwrapNode(nodes, node.id),
              select: node.children[0]?.id ?? null,
            })
          : null,
    },
    {
      label: "Move up",
      icon: <ArrowUp {...icon} />,
      change:
        at && at.index > 0
          ? on((n) => ({ nodes: moveNode(nodes, n.id, -1), select: n.id }))
          : null,
    },
    {
      label: "Move down",
      icon: <ArrowDown {...icon} />,
      change:
        at && at.index < at.count - 1
          ? on((n) => ({ nodes: moveNode(nodes, n.id, 1), select: n.id }))
          : null,
    },
    {
      label: "Delete",
      icon: <Trash2 {...icon} />,
      // A repeat's only child goes with its repeat: an empty repeat is no workout item (the core does not read one).
      change: on((n) => {
        const gone = soleChildOf(nodes, n.id) ?? n;
        return {
          nodes: removeNode(nodes, gone.id),
          select: afterRemoval(nodes, gone.id),
        };
      }),
    },
  ];
}
/** The repeat whose only child is the node with `id`, or null. */
function soleChildOf(
  nodes: readonly WorkoutNode[],
  id: string,
): WorkoutNode | null {
  const p = pathOf(nodes, id);
  const parent = p && p.length > 1 ? nodeAt(nodes, p.slice(0, -1)) : null;
  return parent?.type === "repeat" && parent.children.length === 1
    ? parent
    : null;
}
function findNode(nodes: readonly WorkoutNode[], id: string) {
  const p = pathOf(nodes, id);
  return p === null ? null : nodeAt(nodes, p);
}

/**
 * The outline's toolbar (WAI-ARIA APG toolbar): one tab stop, ← / → / Home / End move between the buttons that apply,
 * Enter or Space runs one. A button that does not apply to the selection is disabled; `blocked` (why the outline
 * cannot be edited now) disables them all and is said under the toolbar.
 */
export function OutlineToolbar({
  actions,
  blocked,
  controls,
  onRun,
}: {
  actions: ReturnType<typeof outlineActions>;
  blocked: string | null;
  /** The outline's id, while there is one (an empty workout has none). */
  controls: string | undefined;
  onRun: (change: (fresh: () => string) => OutlineChange) => void;
}) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const [current, setCurrent] = useState(0);
  const enabled = actions
    .map((a, i) => (a.change && !blocked ? i : -1))
    .filter((i) => i >= 0);
  const stop = enabled.includes(current) ? current : (enabled[0] ?? -1);
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const at = enabled.indexOf(stop);
    const to =
      e.key === "ArrowRight"
        ? enabled[(at + 1) % enabled.length]
        : e.key === "ArrowLeft"
          ? enabled[(at - 1 + enabled.length) % enabled.length]
          : e.key === "Home"
            ? enabled[0]
            : e.key === "End"
              ? enabled.at(-1)
              : undefined;
    if (to === undefined) return;
    e.preventDefault();
    setCurrent(to);
    refs.current[to]?.focus();
  };
  return (
    <>
      <div
        role="toolbar"
        aria-label="Outline actions"
        aria-controls={controls}
        className="editor-outline-tools"
        onKeyDown={onKey}
      >
        {actions.map((a, i) => (
          <button
            key={a.label}
            ref={(el) => {
              refs.current[i] = el;
            }}
            type="button"
            tabIndex={i === stop ? 0 : -1}
            disabled={!a.change || blocked !== null}
            aria-describedby={blocked ? "editor-outline-blocked" : undefined}
            onFocus={() => setCurrent(i)}
            onClick={() => a.change && onRun(a.change)}
          >
            {a.icon}
            {a.label}
          </button>
        ))}
      </div>
      {blocked && (
        <p id="editor-outline-blocked" className="workouts-blocked">
          {blocked}
        </p>
      )}
    </>
  );
}

/**
 * One value field's reading. Today a value is a number; ADR-0008 makes `auto` (a ramp end) and a `$name` variable
 * further readings of the same field, and a number written over a bound value unbinds it (§2.12).
 */
type Reading = { ok: true; value: number } | { ok: false; why: string };
export function readValue(label: string, raw: string): Reading {
  const t = raw.trim();
  const n = Number(t);
  return t !== "" && Number.isFinite(n)
    ? { ok: true, value: n }
    : { ok: false, why: `${label}: enter a number` };
}

/** A form field: its accessible label, unit and value text. */
interface Field {
  key: string;
  label: string;
  unit: string;
}
/** The power fields of a target the form edits; null for one it does not (heart rate, free), which the text edits. */
function powerFields(t: PowerTarget): Field[] | null {
  switch (t.kind) {
    case "pct_ftp":
      return [{ key: "value", label: "Power", unit: "% FTP" }];
    case "watts":
      return [{ key: "value", label: "Power", unit: "W" }];
    case "ramp":
      return [
        { key: "from", label: "Ramp from", unit: "% FTP" },
        { key: "to", label: "Ramp to", unit: "% FTP" },
      ];
    case "range":
      return [
        { key: "lo", label: "Range low", unit: "% FTP" },
        { key: "hi", label: "Range high", unit: "% FTP" },
      ];
    case "hr":
    case "free":
      return null;
  }
}
const str = (n: number | null) => (n === null ? "" : String(n));
/** The form's values for `n`, as the fields show them. */
function valuesOf(n: WorkoutNode): Record<string, string> {
  if (n.type === "repeat") return { label: n.label ?? "", count: str(n.count) };
  const out: Record<string, string> = { label: n.label ?? "" };
  if (n.end.kind === "time" && n.end.s !== null) {
    out.min = String(Math.floor(n.end.s / 60));
    out.sec = String(n.end.s % 60);
  }
  const t = n.target as Record<string, unknown>;
  for (const f of powerFields(n.target) ?? [])
    out[f.key] = str(t[f.key] as number | null);
  return out;
}
/**
 * The node the form's values `v` describe, or why they describe none. A box left as `valuesOf` showed it keeps the
 * node's own value (a null value, shown empty, stays null: ADR-0008's `auto` ends land there). A changed duration is
 * Minutes × 60 + Seconds, neither negative, in whole seconds (0.1 min is 6 s; float noise is rounded away, a fraction
 * of a second is refused).
 */
export function nodeFrom(
  n: WorkoutNode,
  v: Record<string, string>,
): { ok: true; node: WorkoutNode } | { ok: false; why: string } {
  const was = valuesOf(n);
  const changed = (k: string) => (v[k] ?? "") !== (was[k] ?? "");
  const label = !changed("label")
    ? n.label
    : (v.label ?? "").trim() === ""
      ? null
      : v.label!.trim();
  if (n.type === "repeat") {
    if (!changed("count")) return { ok: true, node: { ...n, label } };
    const count = readValue("Count", v.count ?? "");
    if (!count.ok) return count;
    return { ok: true, node: { ...n, label, count: count.value } };
  }
  let end = n.end;
  if (end.kind === "time" && (changed("min") || changed("sec"))) {
    const min = readValue("Minutes", v.min ?? "");
    if (!min.ok) return min;
    const sec = readValue("Seconds", v.sec ?? "");
    if (!sec.ok) return sec;
    if (min.value < 0) return { ok: false, why: "Minutes: not below 0" };
    if (sec.value < 0) return { ok: false, why: "Seconds: not below 0" };
    const total = min.value * 60 + sec.value;
    const whole = Math.round(total);
    if (Math.abs(total - whole) > 1e-6)
      return { ok: false, why: "Duration: whole seconds only" };
    end = { kind: "time", s: whole };
  }
  let target = n.target;
  const fields = powerFields(target);
  if (fields) {
    const t: Record<string, unknown> = { ...target };
    for (const f of fields.filter((f) => changed(f.key))) {
      const r = readValue(f.label, v[f.key] ?? "");
      if (!r.ok) return r;
      t[f.key] = r.value;
    }
    target = t as PowerTarget;
  }
  return { ok: true, node: { ...n, label, end, target } };
}

/**
 * The selected node's form: a step's label, duration (a time end) and power (a flat, watts, range or ramp target), a
 * repeat's label and count. Apply (or Enter in a field) commits the changed node as one undo step; what the form does
 * not edit (another end or target, cadence, a repeat's options) is named, and the text edits it.
 */
export function BlockForm({
  node,
  version,
  blocked,
  onApply,
  onRefuse,
  refusal,
}: {
  node: WorkoutNode;
  /** The saved state the form shows: any change of it (a commit, undo or redo) shows the node's values afresh. */
  version: unknown;
  blocked: string | null;
  /** Commit `next` in place of `node`. */
  onApply: (next: WorkoutNode) => void;
  /** A field that is no number: the form says why, as the core's refusals are said. */
  onRefuse: (why: string) => void;
  /** Why the form's last edit was refused, said under Apply. */
  refusal: ReactNode;
}) {
  const [v, setV] = useState(() => valuesOf(node));
  // A commit, an undo or a redo shows the node's values afresh (values typed and not applied are dropped); a refused
  // edit keeps what was typed. Adjusted while rendering, as React advises for state that follows a prop.
  const [shown, setShown] = useState({ node, version });
  if (shown.node !== node || shown.version !== version) {
    setShown({ node, version });
    setV(valuesOf(node));
  }
  const id = useId();
  // The field's name is its label; its unit is its description.
  const field = (key: string, label: string, unit?: string) => (
    <div key={key} className="editor-field">
      <label htmlFor={`${id}-${key}`}>{label}</label>
      <input
        id={`${id}-${key}`}
        type="text"
        inputMode={key === "label" ? undefined : "decimal"}
        spellCheck={false}
        value={v[key] ?? ""}
        disabled={blocked !== null}
        aria-describedby={unit ? `${id}-${key}-unit` : undefined}
        onChange={(e) => setV({ ...v, [key]: e.target.value })}
      />
      {unit && (
        <span id={`${id}-${key}-unit`} className="editor-unit">
          {unit}
        </span>
      )}
    </div>
  );
  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (blocked) return;
    const r = nodeFrom(node, v);
    if (!r.ok) onRefuse(r.why);
    // Nothing changed: no commit, so no empty undo step.
    else if (JSON.stringify(r.node) !== JSON.stringify(node)) onApply(r.node);
  };
  const isRepeat = node.type === "repeat";
  const fields = isRepeat ? null : powerFields(node.target);
  const notes = isRepeat
    ? [
        (node.progression || node.skipLastRest) &&
          "Its progression and !last are edited in Text.",
      ]
    : [
        node.end.kind !== "time" && "Its end is edited in Text.",
        !fields && "Its target is edited in Text.",
        node.cadence && "Its cadence is edited in Text.",
      ];
  return (
    <form
      className="editor-block"
      aria-labelledby="editor-block-h"
      onSubmit={submit}
    >
      <h3 id="editor-block-h">
        {isRepeat ? "Selected repeat" : "Selected step"}
      </h3>
      {field("label", "Label")}
      {isRepeat ? (
        field("count", "Count", "×")
      ) : (
        <>
          {node.end.kind === "time" && (
            <div className="editor-field-pair">
              {field("min", "Minutes", "min")}
              {field("sec", "Seconds", "s")}
            </div>
          )}
          {fields && (
            <div className="editor-field-pair">
              {fields.map((f) => field(f.key, f.label, f.unit))}
            </div>
          )}
        </>
      )}
      {notes.filter(Boolean).map((n) => (
        <p key={n as string} className="workouts-hint">
          {n}
        </p>
      ))}
      <div className="editor-actions">
        <button type="submit" disabled={blocked !== null}>
          Apply
        </button>
      </div>
      <div aria-live="polite">{refusal}</div>
    </form>
  );
}

/** Replace `node` (by id) in `nodes` by `next`: the form's commit. */
export const withNode = (nodes: readonly WorkoutNode[], next: WorkoutNode) =>
  replaceNode(nodes, next.id, [next]);
