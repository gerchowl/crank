// P13 workout profile renderer, shared by the ride strip, the editor profile, the P24 diff
// overlay and the review background bands. Pure canvas drawing, no state.
import { zoneIndex, type FlatBlock, type Zone } from "@neo/shared";
import type { ChartColors } from "../lib/theme.svelte";
import { zoneColor, withAlpha } from "../lib/theme.svelte";

export interface ProfileGeom {
  x: number;
  y: number;
  w: number;
  h: number;
  t0: number;
  t1: number;
  maxW: number;
}

export interface ProfileStyle {
  colors: ChartColors;
  zones: Zone[];
  /** block indices drawn with the selection outline */
  selected?: (b: FlatBlock, i: number) => boolean;
  /** ghosted (P24 "old" profile): outline only */
  ghost?: boolean;
  /** outline AI-changed blocks */
  showAi?: boolean;
  /** extra outline for changed blocks (P24 diff) */
  changed?: (b: FlatBlock, i: number) => boolean;
  /** dim blocks before this time (ride strip: done) */
  doneBefore?: number;
  gap?: number;
}

export const tx = (g: ProfileGeom, t: number) => g.x + ((t - g.t0) / (g.t1 - g.t0)) * g.w;
export const py = (g: ProfileGeom, w: number) => g.y + g.h - (Math.min(w, g.maxW) / g.maxW) * g.h;

let hatch: { key: string; pattern: CanvasPattern | null } | null = null;

function hatchPattern(ctx: CanvasRenderingContext2D, color: string): CanvasPattern | null {
  const key = color;
  if (hatch?.key === key) return hatch.pattern;
  const c = document.createElement("canvas");
  c.width = 8;
  c.height = 8;
  const g = c.getContext("2d");
  if (!g) return null;
  g.strokeStyle = color;
  g.lineWidth = 1.5;
  g.beginPath();
  g.moveTo(0, 8);
  g.lineTo(8, 0);
  g.moveTo(-2, 2);
  g.lineTo(2, -2);
  g.moveTo(6, 10);
  g.lineTo(10, 6);
  g.stroke();
  hatch = { key, pattern: ctx.createPattern(c, "repeat") };
  return hatch.pattern;
}

export function drawProfile(ctx: CanvasRenderingContext2D, blocks: readonly FlatBlock[], g: ProfileGeom, s: ProfileStyle): void {
  const c = s.colors;
  const gap = s.gap ?? 1;
  blocks.forEach((b, i) => {
    if (b.startS + b.durS < g.t0 || b.startS > g.t1) return;
    const x0 = tx(g, b.startS) + gap / 2;
    const x1 = Math.max(x0 + 1, tx(g, b.startS + b.durS) - gap / 2);
    const yFrom = py(g, b.powerFrom);
    const yTo = py(g, b.powerTo);
    const base = g.y + g.h;
    const mean = (b.powerFrom + b.powerTo) / 2;
    const zi = zoneIndex(s.zones, mean);
    const fill = zoneColor(c, zi, s.zones.length);
    const path = new Path2D();
    path.moveTo(x0, base);
    path.lineTo(x0, yFrom);
    path.lineTo(x1, yTo);
    path.lineTo(x1, base);
    path.closePath();
    if (s.ghost) {
      ctx.strokeStyle = c.muted;
      ctx.lineWidth = 1.5;
      ctx.setLineDash([4, 3]);
      ctx.stroke(path);
      ctx.setLineDash([]);
    } else {
      ctx.globalAlpha = s.doneBefore !== undefined && b.startS + b.durS <= s.doneBefore ? 0.45 : 1;
      ctx.fillStyle = fill;
      ctx.fill(path);
      ctx.globalAlpha = 1;
    }
    // open-ended blocks: hatched range from min to max duration
    if (b.durRangeS && !s.ghost) {
      const [lo, hi] = b.durRangeS;
      const rx0 = tx(g, b.startS + lo);
      const rx1 = tx(g, b.startS + hi);
      const pat = hatchPattern(ctx, fill);
      if (pat && rx1 > rx0) {
        ctx.fillStyle = pat;
        ctx.globalAlpha = 0.9;
        ctx.fillRect(rx0, Math.min(yFrom, yTo), rx1 - rx0, base - Math.min(yFrom, yTo));
        ctx.globalAlpha = 1;
        ctx.strokeStyle = withAlpha(c.ink.startsWith("#") ? c.ink : "#888888", 0.35);
        ctx.lineWidth = 1;
        ctx.strokeRect(rx0 + 0.5, Math.min(yFrom, yTo) + 0.5, rx1 - rx0 - 1, base - Math.min(yFrom, yTo) - 1);
      }
    }
    if (s.showAi && b.aiChanged) {
      ctx.strokeStyle = c.ink;
      ctx.lineWidth = 2;
      ctx.setLineDash([3, 2]);
      ctx.stroke(path);
      ctx.setLineDash([]);
      ctx.fillStyle = c.ink;
      ctx.font = `bold 9px ${c.font}`;
      ctx.fillText("AI", x0 + 2, Math.min(yFrom, yTo) - 3);
    }
    if (s.changed?.(b, i)) {
      ctx.strokeStyle = c.ink;
      ctx.lineWidth = 2;
      ctx.stroke(path);
    }
    if (s.selected?.(b, i)) {
      ctx.strokeStyle = c.accent;
      ctx.lineWidth = 2.5;
      ctx.stroke(path);
    }
  });
}

/** Horizontal guide lines at FTP fractions, labelled in % FTP. */
export function drawFtpGuides(ctx: CanvasRenderingContext2D, g: ProfileGeom, ftp: number, c: ChartColors): void {
  ctx.strokeStyle = c.grid;
  ctx.lineWidth = 1;
  ctx.fillStyle = c.muted;
  ctx.font = `10px ${c.font}`;
  ctx.textBaseline = "middle";
  ctx.textAlign = "right";
  for (const f of [0.5, 1, 1.5]) {
    const y = Math.round(py(g, f * ftp)) + 0.5;
    if (y < g.y) continue;
    ctx.beginPath();
    ctx.moveTo(g.x, y);
    ctx.lineTo(g.x + g.w, y);
    ctx.stroke();
    ctx.fillText(`${f * 100}%`, g.x - 4, y);
  }
  ctx.textAlign = "left";
}
