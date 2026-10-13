// P13 workout profile painter (canvas), shared by the ride strip and the check-in/proposal minis.
import { zoneIndex, type FlatBlock } from "@neo/shared";
import { backend } from "../backend";
import { zoneColor, type ChartTheme } from "../theme/theme";

export function blockZoneColor(t: ChartTheme, b: FlatBlock): string {
  const p = (b.powerFrom + b.powerTo) / 2;
  return zoneColor(t, zoneIndex(backend.zones.power, p), 7);
}

export function hatch(ctx: CanvasRenderingContext2D, color: string): CanvasPattern | null {
  const c = document.createElement("canvas");
  c.width = c.height = 8;
  const g = c.getContext("2d");
  if (!g) return null;
  g.strokeStyle = color;
  g.lineWidth = 1.5;
  g.beginPath();
  g.moveTo(0, 8);
  g.lineTo(8, 0);
  g.stroke();
  return ctx.createPattern(c, "repeat");
}

export interface ProfileOpts {
  /** visible window in workout seconds; default whole workout */
  from?: number;
  to?: number;
  nowS?: number;
  maxW?: number;
  gap?: number;
}

export function paintProfile(ctx: CanvasRenderingContext2D, w: number, h: number, blocks: readonly FlatBlock[], t: ChartTheme, o: ProfileOpts = {}) {
  const last = blocks[blocks.length - 1];
  const total = last ? last.startS + last.durS : 1;
  const from = o.from ?? 0;
  const to = o.to ?? total;
  const maxW = o.maxW ?? Math.max(300, ...blocks.map((b) => Math.max(b.powerFrom, b.powerTo))) * 1.08;
  const x = (s: number) => ((s - from) / (to - from)) * w;
  const y = (p: number) => h - (p / maxW) * h;
  const gap = o.gap ?? 1;
  const hp = hatch(ctx, t.dark ? "rgba(0,0,0,0.45)" : "rgba(255,255,255,0.6)");
  for (const b of blocks) {
    const x0 = x(b.startS);
    const x1 = x(b.startS + b.durS);
    if (x1 < 0 || x0 > w) continue;
    const xa = x0 + gap / 2;
    const xb = Math.max(xa + 0.5, x1 - gap / 2);
    ctx.fillStyle = blockZoneColor(t, b);
    ctx.beginPath();
    ctx.moveTo(xa, h);
    ctx.lineTo(xa, y(b.powerFrom));
    ctx.lineTo(xb, y(b.powerTo));
    ctx.lineTo(xb, h);
    ctx.closePath();
    ctx.fill();
    if (b.durRangeS && hp) {
      ctx.fillStyle = hp;
      ctx.fill();
    }
    if (b.aiChanged) {
      ctx.strokeStyle = t.accent;
      ctx.lineWidth = 2;
      ctx.stroke();
    }
  }
  if (o.nowS !== undefined) {
    const nx = x(o.nowS);
    ctx.fillStyle = t.dark ? "rgba(0,0,0,0.35)" : "rgba(255,255,255,0.45)";
    ctx.fillRect(0, 0, Math.max(0, nx), h);
    ctx.strokeStyle = t.ink;
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(nx, 0);
    ctx.lineTo(nx, h);
    ctx.stroke();
    ctx.fillStyle = t.ink;
    ctx.beginPath();
    ctx.moveTo(nx - 6, 0);
    ctx.lineTo(nx + 6, 0);
    ctx.lineTo(nx, 8);
    ctx.closePath();
    ctx.fill();
  }
}
