import type { HrRecoveryState } from "@neo/shared";
import { drawAxes, ticks, type Box } from "./canvasAxes";
import { alpha } from "./axes";
import type { ChartTheme } from "../theme/theme";

export function paintRecovery(ctx: CanvasRenderingContext2D, w: number, h: number, r: HrRecoveryState, t: ChartTheme) {
  const box: Box = { l: 36, t: 8, r: w - 10, b: h - 20 };
  const t0 = r.startedT;
  const pred = r.predicted ?? [];
  const lastObs = r.observed[r.observed.length - 1]?.t ?? t0;
  const tEnd = Math.max(r.maxS, (pred[pred.length - 1]?.t ?? lastObs) - t0, lastObs - t0 + 10);
  const hrs = r.observed.flatMap((s) => (s.v === null ? [] : [s.v]));
  const yLo = Math.min(r.targetBpm - 8, ...pred.map((p) => p.lo), ...hrs) - 2;
  const yHi = Math.max(r.targetBpm + 10, ...hrs, ...pred.map((p) => p.hi)) + 3;
  const x = (tt: number) => box.l + ((tt - t0) / tEnd) * (box.r - box.l);
  const y = (v: number) => box.b - ((v - yLo) / (yHi - yLo)) * (box.b - box.t);
  drawAxes(ctx, t, box, ticks(0, tEnd, 6).map((s) => ({ v: s, x: x(t0 + s), label: `${String(s)}s` })), ticks(yLo, yHi, 4).map((v) => ({ v, y: y(v), label: String(v) })));

  // min / max duration markers
  ctx.strokeStyle = t.border;
  ctx.setLineDash([2, 4]);
  for (const s of [r.minS, r.maxS]) {
    ctx.beginPath();
    ctx.moveTo(x(t0 + s), box.t);
    ctx.lineTo(x(t0 + s), box.b);
    ctx.stroke();
  }
  // target
  ctx.strokeStyle = t.target;
  ctx.lineWidth = 1.5;
  ctx.setLineDash([6, 4]);
  ctx.beginPath();
  ctx.moveTo(box.l, y(r.targetBpm));
  ctx.lineTo(box.r, y(r.targetBpm));
  ctx.stroke();
  ctx.setLineDash([]);
  ctx.fillStyle = t.ink2;
  ctx.font = `12px ${t.font}`;
  ctx.textAlign = "right";
  ctx.textBaseline = "bottom";
  ctx.fillText(`target ${String(r.targetBpm)} bpm`, box.r - 4, y(r.targetBpm) - 3);

  const hr = t.series.hr;
  // uncertainty band + dotted prediction
  if (pred.length > 1) {
    ctx.fillStyle = alpha(hr, t.dark ? 0.22 : 0.16);
    ctx.beginPath();
    pred.forEach((p, i) => (i ? ctx.lineTo(x(p.t), y(p.hi)) : ctx.moveTo(x(p.t), y(p.hi))));
    for (let i = pred.length - 1; i >= 0; i--) {
      const p = pred[i];
      if (p) ctx.lineTo(x(p.t), y(p.lo));
    }
    ctx.closePath();
    ctx.fill();
    ctx.strokeStyle = hr;
    ctx.lineWidth = 2;
    ctx.setLineDash([2, 4]);
    ctx.beginPath();
    pred.forEach((p, i) => (i ? ctx.lineTo(x(p.t), y(p.mean)) : ctx.moveTo(x(p.t), y(p.mean))));
    ctx.stroke();
    ctx.setLineDash([]);
  }
  // fitted curve over the observed range
  const f = r.fit;
  if (f) {
    ctx.strokeStyle = hr;
    ctx.lineWidth = 2;
    ctx.beginPath();
    for (let tt = r.peakT, i = 0; tt <= lastObs + 1e-6; tt += 1, i++) {
      const v = f.hrInf + f.amp * Math.exp(-(tt - r.peakT) / f.tauS);
      if (i) ctx.lineTo(x(tt), y(v));
      else ctx.moveTo(x(tt), y(v));
    }
    ctx.stroke();
  }
  // observed dots with a surface ring
  for (const s of r.observed) {
    if (s.v === null) continue;
    ctx.beginPath();
    ctx.arc(x(s.t), y(s.v), 3, 0, Math.PI * 2);
    ctx.fillStyle = hr;
    ctx.strokeStyle = t.surface1;
    ctx.lineWidth = 1.5;
    ctx.fill();
    ctx.stroke();
  }
}

