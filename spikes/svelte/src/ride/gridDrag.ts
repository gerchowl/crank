// Edit-mode drag/resize as an attachment: `{@attach gridDrag(widget, "move", grid)}`.
// Mutates the widget's x/y/w/h directly (it's a deep $state proxy), snapped to grid cells;
// only that cell's inline style updates while dragging.
import type { Attachment } from "svelte/attachments";
import type { Widget } from "@neo/shared";

export interface GridInfo {
  el: () => HTMLElement | undefined;
  cols: number;
  rows: () => number;
  onStart?: () => void;
}

export function gridDrag(w: Widget, mode: "move" | "resize", grid: GridInfo): Attachment<HTMLElement> {
  return (handle) => {
    let start: { px: number; py: number; x: number; y: number; w: number; h: number; cw: number; rh: number } | null = null;

    const down = (e: PointerEvent) => {
      const el = grid.el();
      if (!el || e.button !== 0) return;
      e.preventDefault();
      e.stopPropagation();
      const r = el.getBoundingClientRect();
      start = { px: e.clientX, py: e.clientY, x: w.x, y: w.y, w: w.w, h: w.h, cw: r.width / grid.cols, rh: r.height / grid.rows() };
      handle.setPointerCapture(e.pointerId);
      grid.onStart?.();
    };
    const move = (e: PointerEvent) => {
      if (!start) return;
      const dx = Math.round((e.clientX - start.px) / start.cw);
      const dy = Math.round((e.clientY - start.py) / start.rh);
      if (mode === "move") {
        const nx = Math.max(0, Math.min(grid.cols - w.w, start.x + dx));
        const ny = Math.max(0, start.y + dy);
        if (nx !== w.x) w.x = nx;
        if (ny !== w.y) w.y = ny;
      } else {
        const nw = Math.max(1, Math.min(grid.cols - w.x, start.w + dx));
        const nh = Math.max(1, start.h + dy);
        if (nw !== w.w) w.w = nw;
        if (nh !== w.h) w.h = nh;
      }
    };
    const up = (e: PointerEvent) => {
      if (!start) return;
      start = null;
      handle.releasePointerCapture(e.pointerId);
    };
    handle.addEventListener("pointerdown", down);
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", up);
    handle.addEventListener("pointercancel", up);
    return () => {
      handle.removeEventListener("pointerdown", down);
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", up);
      handle.removeEventListener("pointercancel", up);
    };
  };
}
