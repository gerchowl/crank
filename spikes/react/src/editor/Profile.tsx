import { useState } from "react";
import { zoneIndex, type CompiledWorkout, type FlatBlock, type StepNode, type WorkoutNode } from "@neo/shared";
import { backend } from "../backend";
import { useSize } from "../lib/useSize";
import { mmss } from "../lib/format";
import { useEditor } from "./context";
import { pathOf, updateNode } from "./tree";

interface Drag { nodeId: string; kind: "dur" | "power"; startX: number; startY: number; orig: StepNode; block: FlatBlock }

const snap = (s: number) => (s > 120 ? Math.round(s / 15) * 15 : Math.max(5, Math.round(s / 5) * 5));

function pctOf(n: StepNode, ftp: number): number | null {
  const t = n.target;
  switch (t.kind) {
    case "pct_ftp":
      return t.value;
    case "watts":
      return (t.value / ftp) * 100;
    case "range":
      return (t.lo + t.hi) / 2;
    case "ramp":
      return (t.from + t.to) / 2;
    default:
      return null;
  }
}

function withPct(n: StepNode, pct: number, ftp: number): StepNode {
  const t = n.target;
  const p = Math.max(5, Math.round(pct));
  const d = p - (pctOf(n, ftp) ?? p);
  switch (t.kind) {
    case "pct_ftp":
      return { ...n, target: { kind: "pct_ftp", value: p } };
    case "watts":
      return { ...n, target: { kind: "watts", value: Math.round((p / 100) * ftp) } };
    case "range":
      return { ...n, target: { kind: "range", lo: Math.round(t.lo + d), hi: Math.round(t.hi + d) } };
    case "ramp":
      return { ...n, target: { kind: "ramp", from: Math.round(t.from + d), to: Math.round(t.to + d) } };
    default:
      return n;
  }
}

/** P13 editable profile + P24 diff overlay. Blocks: width = time, height = power, zone-ramp fill. */
export function Profile() {
  const { state, dispatch, compiled } = useEditor();
  const [ref, { w }] = useSize();
  const [drag, setDrag] = useState<Drag | null>(null);
  const [preview, setPreview] = useState<WorkoutNode[] | null>(null);
  const ftp = state.workout.ftp;
  const H = 220;
  const pad = { l: 40, r: 8, t: 10, b: 22 };

  const diff = state.diff;
  const shownNodes = preview ?? diff?.after ?? state.nodes;
  const shown: CompiledWorkout = preview || diff ? backend.compileWorkout({ ...state.workout, nodes: shownNodes }) : compiled;
  const ghost = diff ? compiled : null;
  const totalS = Math.max(60, shown.totalS, ghost?.totalS ?? 0);
  const maxW = Math.max(ftp * 1.3, ...shown.blocks.map((b) => Math.max(b.powerFrom, b.powerTo)), ...(ghost?.blocks.map((b) => Math.max(b.powerFrom, b.powerTo)) ?? [])) * 1.05;
  const iw = Math.max(1, w - pad.l - pad.r);
  const ih = H - pad.t - pad.b;
  const x = (s: number) => pad.l + (s / totalS) * iw;
  const y = (p: number) => pad.t + ih - (p / maxW) * ih;

  const selPaths = state.selection.map((id) => pathOf(shownNodes, id)).filter((p) => p !== null);
  const isSel = (b: FlatBlock) => selPaths.some((p) => p.every((v, i) => b.path[i] === v));
  const changed = new Set(diff?.changed ?? []);

  const onMove = (e: React.PointerEvent) => {
    if (!drag) return;
    let next: StepNode = drag.orig;
    if (drag.kind === "dur" && drag.orig.end.kind === "time") {
      const s = snap(drag.block.durS + ((e.clientX - drag.startX) / iw) * totalS);
      next = { ...drag.orig, end: { kind: "time", s } };
    } else if (drag.kind === "power") {
      const p0 = pctOf(drag.orig, ftp) ?? 0;
      const dp = (-(e.clientY - drag.startY) / ih) * (maxW / ftp) * 100;
      next = withPct(drag.orig, p0 + dp, ftp);
    }
    setPreview(updateNode(state.nodes, drag.nodeId, () => next));
  };
  const onUp = () => {
    if (preview) dispatch({ type: "edit", nodes: preview, selection: drag ? [drag.nodeId] : state.selection });
    setDrag(null);
    setPreview(null);
  };
  const startDrag = (e: React.PointerEvent, b: FlatBlock, kind: Drag["kind"]) => {
    const n = findStep(state.nodes, b.nodeId);
    if (!n || diff) return;
    if (kind === "dur" && n.end.kind !== "time") return;
    if (kind === "power" && pctOf(n, ftp) === null) return;
    e.stopPropagation();
    e.currentTarget.setPointerCapture(e.pointerId);
    setDrag({ nodeId: b.nodeId, kind, startX: e.clientX, startY: e.clientY, orig: n, block: b });
  };

  const ticks: number[] = [];
  const step = totalS > 3600 ? 900 : totalS > 1200 ? 300 : 60;
  for (let s = 0; s <= totalS; s += step) ticks.push(s);

  return (
    <div className="profile" ref={ref}>
      {w > 0 && (
        <svg width={w} height={H} onPointerMove={onMove} onPointerUp={onUp} onPointerCancel={onUp} role="img" aria-label="Workout profile">
          <defs>
            <pattern id="hatch" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
              <line x1="0" y1="0" x2="0" y2="6" stroke="var(--surface-0)" strokeWidth="2" strokeOpacity="0.55" />
            </pattern>
          </defs>
          {ticks.map((s) => (
            <g key={s}>
              <line x1={x(s)} x2={x(s)} y1={pad.t} y2={pad.t + ih} stroke="var(--grid)" />
              <text x={x(s)} y={H - 6} className="axis" textAnchor="middle">{mmss(s)}</text>
            </g>
          ))}
          <line x1={pad.l} x2={pad.l + iw} y1={y(ftp)} y2={y(ftp)} stroke="var(--target)" strokeDasharray="6 4" />
          <text x={pad.l - 4} y={y(ftp) + 4} className="axis" textAnchor="end">FTP</text>
          {[0.5, 1.5].map((f) => (
            <text key={f} x={pad.l - 4} y={y(ftp * f) + 4} className="axis" textAnchor="end">{Math.round(ftp * f)}</text>
          ))}
          {shown.blocks.map((b, i) => {
            const x0 = x(b.startS) + 0.5;
            const x1 = Math.max(x0 + 1, x(b.startS + b.durS) - 0.5);
            const d = `M${String(x0)},${String(pad.t + ih)} L${String(x0)},${String(y(b.powerFrom))} L${String(x1)},${String(y(b.powerTo))} L${String(x1)},${String(pad.t + ih)} Z`;
            const z = zoneIndex(backend.zones.power, (b.powerFrom + b.powerTo) / 2);
            const sel = isSel(b);
            return (
              <g key={i} className={`blk ${sel ? "sel" : ""}`}>
                <path
                  d={d}
                  fill={`var(--zone-${String(z + 1)})`}
                  stroke={changed.has(b.nodeId) ? "var(--accent)" : sel ? "var(--ink-primary)" : "none"}
                  strokeWidth={changed.has(b.nodeId) || sel ? 2 : 0}
                  onPointerDown={(e) => {
                    if (e.shiftKey) dispatch({ type: "select", ids: [...state.selection, b.nodeId] });
                    else dispatch({ type: "select", ids: [b.nodeId] });
                    startDrag(e, b, "power");
                  }}
                >
                  <title>{`${mmss(b.durS)} @ ${String(Math.round(b.powerFrom))}${b.powerTo !== b.powerFrom ? `→${String(Math.round(b.powerTo))}` : ""} W (${b.phase}) — drag top: power`}</title>
                </path>
                {b.durRangeS && <path d={d} fill="url(#hatch)" pointerEvents="none" />}
                {b.durRangeS && (
                  <line x1={x(b.startS + b.durRangeS[0])} x2={x(b.startS + b.durRangeS[1])} y1={pad.t + ih - 3} y2={pad.t + ih - 3} stroke="var(--ink-secondary)" strokeWidth={2} pointerEvents="none" />
                )}
                {sel && !diff && (
                  <rect className="edge" x={x1 - 4} y={Math.min(y(b.powerTo), pad.t + ih - 12)} width={8} height={pad.t + ih - Math.min(y(b.powerTo), pad.t + ih - 12)} onPointerDown={(e) => { startDrag(e, b, "dur"); }}>
                    <title>drag: duration</title>
                  </rect>
                )}
              </g>
            );
          })}
          {ghost?.blocks.map((b, i) => (
            <path
              key={`g${String(i)}`}
              d={`M${String(x(b.startS))},${String(pad.t + ih)} L${String(x(b.startS))},${String(y(b.powerFrom))} L${String(x(b.startS + b.durS))},${String(y(b.powerTo))} L${String(x(b.startS + b.durS))},${String(pad.t + ih)}`}
              fill="none"
              stroke="var(--ink-primary)"
              strokeOpacity={0.55}
              strokeDasharray="3 3"
              pointerEvents="none"
            />
          ))}
        </svg>
      )}
      {diff && <div className="diff-legend"><span className="ghost-swatch" /> before (ghost) <span className="new-swatch" /> after <span className="changed-swatch" /> changed by AI</div>}
    </div>
  );
}

function findStep(nodes: readonly WorkoutNode[], id: string): StepNode | undefined {
  for (const n of nodes) {
    if (n.id === id && n.type === "step") return n;
    if (n.type === "repeat") {
      const f = findStep(n.children, id);
      if (f) return f;
    }
  }
  return undefined;
}
