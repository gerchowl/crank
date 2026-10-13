// P13 workout profile — pure geometry + painter shared by the ride strip, editor, check-in
// proposal, diff overlay (P24) and review background bands.
import type { FlatBlock } from "@neo/shared";
import { backend } from "../backend";
import { zoneIdx } from "../lib/format";
import { type ThemeTokens, withAlpha, zoneColor } from "../theme/theme";

export interface ProfileRect {
  block: FlatBlock;
  index: number;
  x0: number;
  x1: number; // expected end
  xMax: number; // range max (open-ended blocks)
  yFrom: number;
  yTo: number;
  base: number;
}

export interface ProfileView {
  t0: number;
  t1: number;
  maxW: number;
  pad: { l: number; r: number; t: number; b: number };
}

export function profileView(blocks: readonly FlatBlock[], opts: Partial<ProfileView> = {}): ProfileView {
  const end = blocks.reduce((m, b) => Math.max(m, b.startS + (b.durRangeS?.[1] ?? b.durS)), 0);
  const maxP = blocks.reduce((m, b) => Math.max(m, b.powerFrom, b.powerTo), 0);
  return {
    t0: opts.t0 ?? 0,
    t1: opts.t1 ?? Math.max(60, end),
    maxW: opts.maxW ?? Math.max(backend.athlete.ftp * 1.3, maxP * 1.08),
    pad: opts.pad ?? { l: 0, r: 0, t: 4, b: 0 },
  };
}

export function layoutProfile(blocks: readonly FlatBlock[], w: number, h: number, v: ProfileView): ProfileRect[] {
  const pw = w - v.pad.l - v.pad.r;
  const ph = h - v.pad.t - v.pad.b;
  const X = (s: number) => v.pad.l + ((s - v.t0) / (v.t1 - v.t0)) * pw;
  const Y = (p: number) => v.pad.t + ph - (Math.min(p, v.maxW) / v.maxW) * ph;
  return blocks.map((b, index) => ({
    block: b,
    index,
    x0: X(b.startS),
    x1: X(b.startS + b.durS),
    xMax: X(b.startS + (b.durRangeS?.[1] ?? b.durS)),
    yFrom: Y(b.powerFrom),
    yTo: Y(b.powerTo),
    base: v.pad.t + ph,
  }));
}

export function xOfTime(t: number, w: number, v: ProfileView): number {
  return v.pad.l + ((t - v.t0) / (v.t1 - v.t0)) * (w - v.pad.l - v.pad.r);
}

export function timeOfX(x: number, w: number, v: ProfileView): number {
  return v.t0 + ((x - v.pad.l) / (w - v.pad.l - v.pad.r)) * (v.t1 - v.t0);
}

export interface PaintOpts {
  hereT?: number;
  selected?: ReadonlySet<string>;
  changed?: ReadonlySet<string>;
  ghost?: readonly ProfileRect[];
  dim?: boolean;
  ftpLine?: boolean;
}

function hatch(ctx: CanvasRenderingContext2D, x0: number, x1: number, y: number, base: number, color: string) {
  ctx.save();
  ctx.beginPath();
  ctx.rect(x0, y, x1 - x0, base - y);
  ctx.clip();
  ctx.strokeStyle = color;
  ctx.lineWidth = 1;
  for (let x = x0 - (base - y); x < x1; x += 6) {
    ctx.beginPath();
    ctx.moveTo(x, base);
    ctx.lineTo(x + (base - y), y);
    ctx.stroke();
  }
  ctx.restore();
}

export function paintProfile(ctx: CanvasRenderingContext2D, rects: readonly ProfileRect[], t: ThemeTokens, w: number, v: ProfileView, o: PaintOpts = {}) {
  const zones = backend.zones.power;
  const gap = rects.length > 120 ? 0 : 1;
  // ghost (P24: old profile behind the new one)
  if (o.ghost) {
    ctx.fillStyle = withAlpha(t.muted, 0.22);
    ctx.strokeStyle = t.muted;
    ctx.setLineDash([3, 3]);
    for (const r of o.ghost) {
      ctx.beginPath();
      ctx.moveTo(r.x0, r.base);
      ctx.lineTo(r.x0, r.yFrom);
      ctx.lineTo(r.x1, r.yTo);
      ctx.lineTo(r.x1, r.base);
      ctx.fill();
      ctx.stroke();
    }
    ctx.setLineDash([]);
  }
  for (const r of rects) {
    if (r.x1 < 0 || r.x0 > w) continue;
    const mid = (r.block.powerFrom + r.block.powerTo) / 2;
    const fill = zoneColor(t, zoneIdx(zones, mid));
    const x0 = r.x0 + (r.x1 - r.x0 > 3 ? gap / 2 : 0);
    const x1 = r.x1 - (r.x1 - r.x0 > 3 ? gap / 2 : 0);
    ctx.globalAlpha = o.dim ? 0.55 : 1;
    ctx.fillStyle = fill;
    ctx.beginPath();
    ctx.moveTo(x0, r.base);
    ctx.lineTo(x0, r.yFrom);
    ctx.lineTo(x1, r.yTo);
    ctx.lineTo(x1, r.base);
    ctx.closePath();
    ctx.fill();
    // open-ended range: hatched texture to the range max, tick at the expected end
    if (r.xMax > r.x1 + 1) hatch(ctx, r.x1, r.xMax, r.yTo, r.base, fill);
    ctx.globalAlpha = 1;
    const id = r.block.nodeId;
    if (o.changed?.has(id) || r.block.aiChanged) {
      ctx.strokeStyle = t.ink;
      ctx.lineWidth = 2;
      ctx.setLineDash([4, 2]);
      ctx.strokeRect(x0 + 1, Math.min(r.yFrom, r.yTo) + 1, Math.max(1, x1 - x0 - 2), r.base - Math.min(r.yFrom, r.yTo) - 2);
      ctx.setLineDash([]);
    }
    if (o.selected?.has(id)) {
      ctx.strokeStyle = t.accent;
      ctx.lineWidth = 2;
      ctx.strokeRect(x0 + 1, Math.min(r.yFrom, r.yTo) + 1, Math.max(1, x1 - x0 - 2), r.base - Math.min(r.yFrom, r.yTo) - 2);
    }
  }
  if (o.ftpLine && rects[0]) {
    const ph = rects[0].base - v.pad.t;
    const y = Math.round(v.pad.t + ph - (backend.athlete.ftp / v.maxW) * ph) + 0.5;
    ctx.strokeStyle = t.target;
    ctx.setLineDash([6, 4]);
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(v.pad.l, y);
    ctx.lineTo(w - v.pad.r, y);
    ctx.stroke();
    ctx.setLineDash([]);
    ctx.fillStyle = t.muted;
    ctx.font = `10px ${t.font}`;
    ctx.fillText("FTP", w - v.pad.r - 24, y - 3);
  }
  if (o.hereT !== undefined) {
    const x = Math.round(xOfTime(o.hereT, w, v)) + 0.5;
    const base = rects[0]?.base ?? 0;
    ctx.strokeStyle = t.ink;
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(x, v.pad.t);
    ctx.lineTo(x, base);
    ctx.stroke();
    ctx.fillStyle = t.ink;
    ctx.beginPath();
    ctx.moveTo(x, v.pad.t - 2);
    ctx.lineTo(x + 6, v.pad.t + 5);
    ctx.lineTo(x, v.pad.t + 12);
    ctx.lineTo(x - 6, v.pad.t + 5);
    ctx.closePath();
    ctx.fill();
  }
}
