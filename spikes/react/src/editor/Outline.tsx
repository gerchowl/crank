import { useState } from "react";
import { zoneIndex, type RepeatNode, type StepNode, type WorkoutNode } from "@neo/shared";
import { backend } from "../backend";
import { useEditor } from "./context";
import { move, siblingRun, stepWithoutLabel, updateNode, type DropPos } from "./tree";

interface DropHint { id: string; pos: DropPos }

/** Outline: indented tree, collapse, drag-to-reorder (into/out of repeats), inline field edits. */
export function Outline() {
  const { state, dispatch } = useEditor();
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(new Set());
  const [hint, setHint] = useState<DropHint | null>(null);
  const [dragIds, setDragIds] = useState<string[]>([]);
  const nodes = state.diff?.after ?? state.nodes;
  const readOnly = !!state.diff;
  const changed = new Set(state.diff?.changed ?? []);

  const select = (e: React.MouseEvent, id: string) => {
    if (e.metaKey || e.ctrlKey) {
      dispatch({ type: "select", ids: state.selection.includes(id) ? state.selection.filter((x) => x !== id) : [...state.selection, id] });
    } else if (e.shiftKey && state.selection.length) {
      // extend to a contiguous sibling run when possible
      const run = siblingRun(state.nodes, [...state.selection, id]);
      if (run) {
        const parent = run.parent.reduce<readonly WorkoutNode[]>((list, i) => {
          const n = list[i];
          return n?.type === "repeat" ? n.children : [];
        }, state.nodes);
        dispatch({ type: "select", ids: parent.slice(run.from, run.to + 1).map((n) => n.id) });
      } else dispatch({ type: "select", ids: [...state.selection, id] });
    } else dispatch({ type: "select", ids: [id] });
  };

  const row = (n: WorkoutNode, depth: number) => {
    const sel = state.selection.includes(n.id);
    const cls = `orow ${sel ? "sel" : ""} ${changed.has(n.id) ? "changed" : ""} ${hint?.id === n.id ? `drop-${hint.pos}` : ""}`;
    const dnd = readOnly
      ? {}
      : {
          draggable: true,
          onDragStart: (e: React.DragEvent) => {
            const ids = sel ? state.selection : [n.id];
            setDragIds(ids);
            e.dataTransfer.setData("text/plain", ids.join(","));
            e.dataTransfer.effectAllowed = "move";
          },
          onDragOver: (e: React.DragEvent) => {
            if (!dragIds.length) return;
            e.preventDefault();
            e.stopPropagation();
            const r = e.currentTarget.getBoundingClientRect();
            const f = (e.clientY - r.top) / r.height;
            const pos: DropPos = n.type === "repeat" && f > 0.3 && f < 0.7 ? "into" : f < 0.5 ? "before" : "after";
            if (hint?.id !== n.id || hint.pos !== pos) setHint({ id: n.id, pos });
          },
          onDragLeave: () => {
            setHint(null);
          },
          onDrop: (e: React.DragEvent) => {
            e.preventDefault();
            e.stopPropagation();
            if (hint) {
              const out = move(state.nodes, dragIds, hint.id, hint.pos);
              if (out) dispatch({ type: "edit", nodes: out, selection: dragIds });
            }
            setHint(null);
            setDragIds([]);
          },
          onDragEnd: () => {
            setHint(null);
            setDragIds([]);
          },
        };
    if (n.type === "repeat") {
      const open = !collapsed.has(n.id);
      return (
        <li key={n.id} role="treeitem" aria-expanded={open} aria-selected={sel}>
          <div className={cls} style={{ paddingLeft: depth * 18 + 4 }} onClick={(e) => { select(e, n.id); }} {...dnd}>
            <span className="grip" aria-hidden>⋮⋮</span>
            <button
              type="button"
              className="icon-btn twisty"
              aria-label={open ? "collapse" : "expand"}
              onClick={(e) => {
                e.stopPropagation();
                const next = new Set(collapsed);
                if (open) next.add(n.id);
                else next.delete(n.id);
                setCollapsed(next);
              }}
            >
              {open ? "▾" : "▸"}
            </button>
            <RepeatFields node={n} readOnly={readOnly} />
          </div>
          {open && <ul role="group">{n.children.map((c) => row(c, depth + 1))}</ul>}
        </li>
      );
    }
    return (
      <li key={n.id} role="treeitem" aria-selected={sel}>
        <div className={cls} style={{ paddingLeft: depth * 18 + 4 }} onClick={(e) => { select(e, n.id); }} {...dnd}>
          <span className="grip" aria-hidden>⋮⋮</span>
          <StepFields node={n} readOnly={readOnly} />
        </div>
      </li>
    );
  };

  return (
    <ul className="outline" role="tree" aria-label="Workout outline" aria-multiselectable>
      {nodes.map((n) => row(n, 0))}
    </ul>
  );
}

function useCommit() {
  const { state, dispatch } = useEditor();
  return (id: string, f: (n: WorkoutNode) => WorkoutNode) => {
    dispatch({ type: "edit", nodes: updateNode(state.nodes, id, f) });
  };
}

function LabelInput({ node, readOnly }: { node: WorkoutNode; readOnly: boolean }) {
  const commit = useCommit();
  return (
    <input
      key={node.label ?? ""}
      className="olabel"
      defaultValue={node.label ?? ""}
      placeholder="+ label"
      readOnly={readOnly}
      aria-label="label"
      onClick={(e) => { e.stopPropagation(); }}
      onKeyDown={(e) => { if (e.key === "Enter") e.currentTarget.blur(); }}
      onBlur={(e) => {
        const v = e.target.value.trim();
        if (v === (node.label ?? "")) return;
        commit(node.id, (n) => {
          const copy = { ...n };
          if (v) copy.label = v;
          else delete copy.label;
          return copy;
        });
      }}
    />
  );
}

/** Step fields are edited as their DSL spec (e.g. `4m 95% 90-95rpm`) — one parser, one source of truth. */
function StepFields({ node, readOnly }: { node: StepNode; readOnly: boolean }) {
  const { state } = useEditor();
  const commit = useCommit();
  const spec = backend.printDsl([stepWithoutLabel(node)]);
  const [err, setErr] = useState<string | null>(null);
  const c = backend.compileWorkout({ ...state.workout, nodes: [node] }).blocks[0];
  const z = c ? zoneIndex(backend.zones.power, (c.powerFrom + c.powerTo) / 2) : 0;
  return (
    <>
      <span className="zone-dot" style={{ background: `var(--zone-${String(z + 1)})` }} title={`Z${String(z + 1)}`} />
      <LabelInput node={node} readOnly={readOnly} />
      <input
        key={spec}
        className={`ospec ${err ? "bad" : ""}`}
        defaultValue={spec}
        readOnly={readOnly}
        aria-label="step"
        title={err ?? "duration / kJ / lap / hr<N min..max, power, cadence"}
        onClick={(e) => { e.stopPropagation(); }}
        onKeyDown={(e) => { if (e.key === "Enter") e.currentTarget.blur(); }}
        onBlur={(e) => {
          const text = e.target.value.trim();
          if (text === spec) {
            setErr(null);
            return;
          }
          const r = backend.parseDsl(text, state.workout.ftp);
          const parsed = "nodes" in r ? r.nodes[0] : undefined;
          if ("error" in r || parsed?.type !== "step" || r.nodes.length !== 1) {
            setErr("error" in r ? r.error.message : "one step expected");
            return;
          }
          setErr(null);
          commit(node.id, () => ({ ...parsed, id: node.id, ...(node.label ? { label: node.label } : {}) }));
        }}
      />
      {c && <span className="muted small">{Math.round(c.powerFrom)}{c.powerTo !== c.powerFrom ? `→${String(Math.round(c.powerTo))}` : ""} W</span>}
    </>
  );
}

function RepeatFields({ node, readOnly }: { node: RepeatNode; readOnly: boolean }) {
  const commit = useCommit();
  const set = (patch: Partial<RepeatNode>) => {
    commit(node.id, (n) => (n.type === "repeat" ? { ...n, ...patch } : n));
  };
  return (
    <>
      <input
        key={`c${String(node.count)}`}
        type="number"
        className="ocount"
        min={1}
        max={50}
        defaultValue={node.count}
        readOnly={readOnly}
        aria-label="repeat count"
        onClick={(e) => { e.stopPropagation(); }}
        onBlur={(e) => {
          const v = Math.max(1, Math.min(50, Math.round(Number(e.target.value))));
          if (v !== node.count) set({ count: v });
        }}
      />
      <span className="times">×</span>
      <LabelInput node={node} readOnly={readOnly} />
      <label className="inline small" onClick={(e) => { e.stopPropagation(); }}>
        +W/rep
        <input
          key={`p${String(node.progression?.wattsPerRep ?? 0)}`}
          type="number"
          className="oprog"
          defaultValue={node.progression?.wattsPerRep ?? 0}
          readOnly={readOnly}
          onBlur={(e) => {
            const v = Math.round(Number(e.target.value));
            if (v === (node.progression?.wattsPerRep ?? 0)) return;
            const copy: RepeatNode = { ...node };
            if (v) copy.progression = { ...node.progression, wattsPerRep: v };
            else delete copy.progression;
            commit(node.id, () => copy);
          }}
        />
      </label>
      <label className="inline small" title="skip the last rest" onClick={(e) => { e.stopPropagation(); }}>
        <input type="checkbox" checked={!!node.skipLastRest} disabled={readOnly} onChange={(e) => { set({ skipLastRest: e.target.checked }); }} /> !last
      </label>
    </>
  );
}
