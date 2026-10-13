import type { ChartTheme } from "../theme/theme";

export interface Box { l: number; t: number; r: number; b: number }

/** Recessive axis labels + hairline grid for plain-canvas plots. */
export function drawAxes(ctx: CanvasRenderingContext2D, t: ChartTheme, box: Box, xt: { v: number; x: number; label: string }[], yt: { v: number; y: number; label: string }[]) {
  ctx.font = `11px ${t.font}`;
  ctx.fillStyle = t.muted;
  ctx.strokeStyle = t.grid;
  ctx.lineWidth = 1;
  ctx.textAlign = "center";
  ctx.textBaseline = "top";
  for (const k of xt) ctx.fillText(k.label, k.x, box.b + 4);
  ctx.textAlign = "right";
  ctx.textBaseline = "middle";
  for (const k of yt) {
    ctx.beginPath();
    ctx.moveTo(box.l, Math.round(k.y) + 0.5);
    ctx.lineTo(box.r, Math.round(k.y) + 0.5);
    ctx.stroke();
    ctx.fillText(k.label, box.l - 4, k.y);
  }
}

export function niceStep(range: number, target: number): number {
  const raw = range / Math.max(1, target);
  const p = Math.pow(10, Math.floor(Math.log10(raw)));
  const n = raw / p;
  return (n < 1.5 ? 1 : n < 3.5 ? 2 : n < 7.5 ? 5 : 10) * p;
}

export function ticks(lo: number, hi: number, target: number): number[] {
  const s = niceStep(hi - lo, target);
  const out: number[] = [];
  for (let v = Math.ceil(lo / s) * s; v <= hi + 1e-9; v += s) out.push(Math.round(v * 1e6) / 1e6);
  return out;
}

/** Interpolate along a ramp of hex colours, f in [0,1]. */
export function rampAt(ramp: readonly string[], f: number): string {
  const n = ramp.length;
  if (!n) return "#888";
  const x = Math.max(0, Math.min(1, f)) * (n - 1);
  const i = Math.floor(x);
  const a = ramp[i] ?? "#888";
  const b = ramp[Math.min(n - 1, i + 1)] ?? a;
  const k = x - i;
  const pa = parseInt(a.slice(1), 16);
  const pb = parseInt(b.slice(1), 16);
  const ch = (sh: number) => Math.round(((pa >> sh) & 255) * (1 - k) + ((pb >> sh) & 255) * k);
  return `rgb(${String(ch(16))},${String(ch(8))},${String(ch(0))})`;
}
