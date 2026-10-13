import { createSignal, For, Match, Show, Switch } from "solid-js";
import type { RepeatNode, StepNode, WorkoutNode } from "@neo/shared";
import { backend, ftp } from "../backend";
import { editor } from "./state";

type Where = "before" | "after" | "into";
const [dragId, setDragId] = createSignal<string | null>(null);
const [dropAt, setDropAt] = createSignal<{ id: string; where: Where } | null>(null);

/** Text of a step without its label, e.g. `4m 95% 90-95rpm` (edited inline via the DSL). */
function stepBody(n: StepNode): string {
  const { label: _label, ...rest } = n;
  return backend.printDsl([rest]);
}

/** Outline: indented tree, collapse, drag-to-reorder (incl. into/out of repeats), inline edit. */
export function Outline(props: { nodes: readonly WorkoutNode[]; readOnly?: boolean; changed?: ReadonlySet<string>; depth?: number }) {
  return (
    <ul class="outline" role={props.depth ? "group" : "tree"}>
      <For each={props.nodes}>
        {(n) => (
          <Row node={n} depth={props.depth ?? 0} readOnly={props.readOnly ?? false} changed={props.changed} />
        )}
      </For>
    </ul>
  );
}

function Row(props: { node: WorkoutNode; depth: number; readOnly: boolean; changed: ReadonlySet<string> | undefined }) {
  const selected = () => editor.selection().has(props.node.id);
  const isCollapsed = () => editor.collapsed().has(props.node.id);
  const drop = () => {
    const d = dropAt();
    return d?.id === props.node.id ? d.where : null;
  };

  const onDragOver = (e: DragEvent) => {
    const src = dragId();
    if (props.readOnly || !src || src === props.node.id) return;
    e.preventDefault();
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const f = (e.clientY - r.top) / r.height;
    const where: Where = props.node.type === "repeat" && f > 0.3 && f < 0.7 ? "into" : f < 0.5 ? "before" : "after";
    setDropAt({ id: props.node.id, where });
  };
  const onDrop = (e: DragEvent) => {
    e.preventDefault();
    const src = dragId();
    const d = dropAt();
    if (src && d) editor.actions.move(src, d.id, d.where);
    setDragId(null);
    setDropAt(null);
  };

  return (
    <li role="treeitem" aria-selected={selected()} aria-expanded={props.node.type === "repeat" ? !isCollapsed() : undefined}>
      <div
        class="ol-row"
        classList={{
          selected: selected(),
          changed: props.changed?.has(props.node.id) ?? false,
          repeat: props.node.type === "repeat",
          "drop-before": drop() === "before",
          "drop-after": drop() === "after",
          "drop-into": drop() === "into",
        }}
        style={{ "padding-left": `${props.depth * 18 + 4}px` }}
        draggable={!props.readOnly}
        onDragStart={(e) => {
          setDragId(props.node.id);
          e.dataTransfer?.setData("text/plain", props.node.id);
        }}
        onDragEnd={() => {
          setDragId(null);
          setDropAt(null);
        }}
        onDragOver={onDragOver}
        onDragLeave={() => {
          if (dropAt()?.id === props.node.id) setDropAt(null);
        }}
        onDrop={onDrop}
        onClick={(e) => {
          if (!props.readOnly) editor.actions.select(props.node.id, e.shiftKey || e.metaKey || e.ctrlKey);
        }}
      >
        <span class="ol-grip" aria-hidden="true">
          ⋮⋮
        </span>
        <Switch>
          <Match when={props.node.type === "repeat" && props.node}>
            {(r) => <RepeatRow node={r()} readOnly={props.readOnly} collapsed={isCollapsed()} />}
          </Match>
          <Match when={props.node.type === "step" && props.node}>{(s) => <StepRow node={s()} readOnly={props.readOnly} />}</Match>
        </Switch>
      </div>
      <Show when={props.node.type === "repeat" && !isCollapsed() && props.node}>
        {(r) => <Outline nodes={r().children} depth={props.depth + 1} readOnly={props.readOnly} {...(props.changed ? { changed: props.changed } : {})} />}
      </Show>
    </li>
  );
}

function LabelInput(props: { node: WorkoutNode; readOnly: boolean }) {
  return (
    <input
      class="ol-label"
      placeholder="label"
      value={props.node.label ?? ""}
      disabled={props.readOnly}
      onChange={(e) => {
        const v = e.currentTarget.value.trim();
        editor.actions.update(props.node.id, (n) => {
          const { label: _l, ...rest } = n;
          return v ? { ...rest, label: v } : rest;
        });
      }}
    />
  );
}

function RepeatRow(props: { node: RepeatNode; readOnly: boolean; collapsed: boolean }) {
  return (
    <>
      <button
        class="ol-twisty"
        aria-label={props.collapsed ? "Expand" : "Collapse"}
        onClick={(e) => {
          e.stopPropagation();
          editor.actions.toggleCollapsed(props.node.id);
        }}
      >
        {props.collapsed ? "▸" : "▾"}
      </button>
      <input
        class="num ol-count"
        type="number"
        min="1"
        max="50"
        value={props.node.count}
        disabled={props.readOnly}
        aria-label="Repeat count"
          onChange={(e) => {
          const c = Math.max(1, Math.min(50, Math.round(Number(e.currentTarget.value) || 1)));
          editor.actions.update(props.node.id, (n) => (n.type === "repeat" ? { ...n, count: c } : n));
        }}
      />
      <span class="ol-x">×</span>
      <LabelInput node={props.node} readOnly={props.readOnly} />
      <label class="ol-opt" title="Progression: watts added per rep to work steps">
        +
        <input
          class="num tiny"
          type="number"
          step="5"
          value={props.node.progression?.wattsPerRep ?? 0}
          disabled={props.readOnly}
          onChange={(e) => {
            const w = Math.round(Number(e.currentTarget.value) || 0);
            editor.actions.update(props.node.id, (n) => {
              if (n.type !== "repeat") return n;
              const { progression: _p, ...rest } = n;
              return w ? { ...rest, progression: { ...n.progression, wattsPerRep: w } } : rest;
            });
          }}
        />
        W/rep
      </label>
      <label class="ol-opt" title="Skip the last rest on the final rep">
        <input
          type="checkbox"
          checked={props.node.skipLastRest ?? false}
          disabled={props.readOnly}
          onChange={(e) => {
            const v = e.currentTarget.checked;
            editor.actions.update(props.node.id, (n) => {
              if (n.type !== "repeat") return n;
              const { skipLastRest: _s, ...rest } = n;
              return v ? { ...rest, skipLastRest: true } : rest;
            });
          }}
        />
        !last
      </label>
    </>
  );
}

function StepRow(props: { node: StepNode; readOnly: boolean }) {
  const [err, setErr] = createSignal<string | null>(null);
  const kind = () => (props.node.end.kind === "hr_recovery" ? "hr<" : props.node.end.kind);
  return (
    <>
      <span class={`ol-kind k-${props.node.end.kind}`}>{kind()}</span>
      <LabelInput node={props.node} readOnly={props.readOnly} />
      <input
        class="ol-body"
        classList={{ invalid: err() !== null }}
        title={err() ?? "Edit as DSL, e.g. 4m 95% 90rpm · hr<118 2m..4m 40% · 10kj 250w"}
        value={stepBody(props.node)}
        disabled={props.readOnly}
          onChange={(e) => {
          const r = backend.parseDsl(e.currentTarget.value, ftp);
          const first = "nodes" in r ? r.nodes[0] : undefined;
          if ("error" in r || r.nodes.length !== 1 || first?.type !== "step") {
            setErr("error" in r ? r.error.message : "Expected exactly one step");
            return;
          }
          setErr(null);
          editor.actions.update(props.node.id, (n) => {
            if (n.type !== "step") return n;
            const next: StepNode = { type: "step", id: n.id, end: first.end, target: first.target };
            if (n.label) next.label = n.label;
            if (first.cadence) next.cadence = first.cadence;
            return next;
          });
        }}
      />
    </>
  );
}
