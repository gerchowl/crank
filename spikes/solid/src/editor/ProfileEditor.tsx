import { createMemo, createSignal } from "solid-js";
import type { CompiledWorkout, FlatBlock, PowerTarget, StepNode } from "@neo/shared";
import { backend, ftp } from "../backend";
import { Canvas, type Hover } from "../charts/Canvas";
import { layoutProfile, paintProfile, type ProfileRect, type ProfileView, profileView, timeOfX } from "../charts/profile";
import { clock, formatTarget } from "../lib/format";
import type { ThemeTokens } from "../theme/theme";
import { editor } from "./state";
import { findById } from "./tree";

const PAD = { l: 40, r: 10, t: 12, b: 20 };
const EDGE = 7;

interface DragState {
  kind: "duration" | "power";
  id: string;
  block: FlatBlock;
  view: ProfileView; // frozen during the drag so the axis doesn't move under the pointer
  w: number;
  h: number;
}

function shiftPower(t: PowerTarget, pct: number): PowerTarget | null {
  const r1 = (x: number) => Math.round(x);
  switch (t.kind) {
    case "pct_ftp":
      return { kind: "pct_ftp", value: r1(pct) };
    case "watts":
      return { kind: "watts", value: Math.round((pct / 100) * ftp) };
    case "range": {
      const mid = (t.lo + t.hi) / 2;
      return { kind: "range", lo: r1(t.lo + pct - mid), hi: r1(t.hi + pct - mid) };
    }
    case "ramp": {
      const top = Math.max(t.from, t.to);
      return { kind: "ramp", from: r1(t.from + pct - top), to: r1(t.to + pct - top) };
    }
    case "hr":
    case "free":
      return null;
  }
}

/** P13 in the editor: blocks ∝ time, height = power; drag right edge = duration, top = power. P24 diff overlay. */
export function ProfileEditor() {
  const [size, setSize] = createSignal({ w: 0, h: 0 });
  const [cursor, setCursor] = createSignal("default");
  const [drag, setDrag] = createSignal<DragState | null>(null);

  const diff = () => editor.diff();
  const compiled = (): CompiledWorkout => editor.preview();
  const ghost = createMemo(() => {
    const d = diff();
    return d ? backend.compileWorkout({ ...editor.doc, nodes: d.before }).blocks : null;
  });

  const view = (): ProfileView => {
    const dr = drag();
    if (dr) return dr.view;
    const g = ghost();
    const all = g ? [...compiled().blocks, ...g] : compiled().blocks;
    return profileView(all, { pad: PAD });
  };

  const rects = (w: number, h: number): ProfileRect[] => layoutProfile(compiled().blocks, w, h, view());

  const draw = (ctx: CanvasRenderingContext2D, w: number, h: number, t: ThemeTokens) => {
    setSize({ w, h });
    const v = view();
    paintAxes(ctx, w, h, t, v);
    const g = ghost();
    paintProfile(ctx, rects(w, h), t, w, v, {
      selected: editor.selection(),
      ftpLine: true,
      ...(g ? { ghost: layoutProfile(g, w, h, v), changed: diff()?.changed ?? new Set<string>() } : {}),
    });
  };

  const hit = (p: Hover): { rect: ProfileRect; zone: "edge" | "top" | "body" } | null => {
    const { w, h } = size();
    for (const r of rects(w, h)) {
      const top = Math.min(r.yFrom, r.yTo);
      if (p.x >= r.x0 && p.x <= r.x1 + EDGE && p.y >= top - EDGE && p.y <= r.base) {
        if (Math.abs(p.x - r.x1) <= EDGE && p.y > top + 4) return { rect: r, zone: "edge" };
        if (Math.abs(p.y - top) <= EDGE) return { rect: r, zone: "top" };
        if (p.x <= r.x1) return { rect: r, zone: "body" };
      }
    }
    return null;
  };

  const local = (e: PointerEvent): Hover => {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  };

  const onDown = (e: PointerEvent) => {
    if (diff()) return;
    const p = local(e);
    const h = hit(p);
    if (!h) {
      editor.actions.clearSelection();
      return;
    }
    const id = h.rect.block.nodeId;
    editor.actions.select(id, e.shiftKey || e.metaKey || e.ctrlKey);
    const node = findById(editor.nodes(), id);
    if (node?.type !== "step") return;
    if (h.zone === "edge" && node.end.kind === "time") {
      setDrag({ kind: "duration", id, block: h.rect.block, view: view(), ...dims() });
    } else if (h.zone === "top" && shiftPower(node.target, 100) !== null) {
      setDrag({ kind: "power", id, block: h.rect.block, view: view(), ...dims() });
    } else return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  };
  const dims = () => ({ w: size().w, h: size().h });

  const onMove = (e: PointerEvent) => {
    const p = local(e);
    const d = drag();
    if (!d) {
      const h = diff() ? null : hit(p);
      setCursor(h?.zone === "edge" ? "ew-resize" : h?.zone === "top" ? "ns-resize" : h ? "pointer" : "default");
      return;
    }
    if (d.kind === "duration") {
      const t = timeOfX(p.x, d.w, d.view);
      const s = Math.max(5, Math.round((t - d.block.startS) / 5) * 5);
      editor.actions.update(d.id, (n) => (n.type === "step" ? { ...n, end: { kind: "time", s } } : n), `drag-${d.id}`);
    } else {
      const ph = d.h - d.view.pad.t - d.view.pad.b;
      const watts = ((d.view.pad.t + ph - p.y) / ph) * d.view.maxW;
      const pct = Math.max(20, Math.min(300, Math.round((watts / ftp) * 100)));
      editor.actions.update(
        d.id,
        (n) => {
          if (n.type !== "step") return n;
          const next = shiftPower(n.target, pct);
          return next ? ({ ...n, target: next } satisfies StepNode) : n;
        },
        `drag-${d.id}`,
      );
    }
  };
  const onUp = () => setDrag(null);

  const tooltip = (p: Hover) => {
    const h = hit(p);
    if (!h) return null;
    const b = h.rect.block;
    const node = findById(diff()?.after ?? editor.nodes(), b.nodeId);
    return (
      <>
        <b>
          {node?.label ? `${node.label} · ` : ""}
          {clock(b.durS)}
          {b.durRangeS ? ` (${clock(b.durRangeS[0])}–${clock(b.durRangeS[1])})` : ""}
        </b>
        <div>
          {node?.type === "step" ? formatTarget(node.target) : ""} · {Math.round(b.powerFrom)}
          {b.powerTo !== b.powerFrom ? `→${Math.round(b.powerTo)}` : ""} W
        </div>
        <div class="muted">
          {b.phase}
          {h.zone === "edge" ? " · drag to change duration" : h.zone === "top" ? " · drag to change power" : ""}
        </div>
      </>
    );
  };

  return (
    <div class="profile-editor" style={{ cursor: cursor() }} onPointerDown={onDown} onPointerMove={onMove} onPointerUp={onUp} onPointerCancel={onUp}>
      <Canvas draw={draw} tooltip={tooltip} ariaLabel="workout profile editor" />
    </div>
  );
}

function paintAxes(ctx: CanvasRenderingContext2D, w: number, h: number, t: ThemeTokens, v: ProfileView) {
  ctx.font = `11px ${t.font}`;
  ctx.fillStyle = t.muted;
  ctx.strokeStyle = t.grid;
  ctx.lineWidth = 1;
  const ph = h - v.pad.t - v.pad.b;
  for (let pct = 50; pct <= (v.maxW / ftp) * 100; pct += 50) {
    const y = Math.round(v.pad.t + ph - ((pct / 100) * ftp * ph) / v.maxW) + 0.5;
    ctx.beginPath();
    ctx.moveTo(v.pad.l, y);
    ctx.lineTo(w - v.pad.r, y);
    ctx.stroke();
    ctx.fillText(`${pct}%`, 4, y + 4);
  }
  const span = v.t1 - v.t0;
  const step = span > 3600 ? 900 : span > 1200 ? 300 : 60;
  for (let s = 0; s <= span; s += step) {
    const x = v.pad.l + (s / span) * (w - v.pad.l - v.pad.r);
    ctx.fillText(clock(s), x - 10, h - 4);
  }
}
