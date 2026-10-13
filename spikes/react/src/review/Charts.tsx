import type { HrRecoveryState } from "@neo/shared";
import { useCanvas } from "../charts/useCanvas";
import { drawAxes, ticks, type Box } from "../charts/canvasAxes";
import { useChartTheme } from "../theme/theme";
import type { RideRecord } from "./record";

/** P16 — power × HR points, colour = kJ spent (fatigue ramp, sequential; hue ≠ zones). */
export function FatigueScatter({ rec }: { rec: RideRecord }) {
  const theme = useChartTheme();
  const { ref } = useCanvas((ctx, w, h) => {
    const P = rec.series["power"] ?? [];
    const H = rec.series["hr"] ?? [];
    const K = rec.series["kj"] ?? [];
    const box: Box = { l: 34, t: 8, r: w - 8, b: h - 18 };
    const xMax = Math.max(100, ...P.map((v) => v ?? 0)) * 1.05;
    const hrs = H.filter((v): v is number => v !== null);
    const yLo = Math.min(...hrs, 90) - 5;
    const yHi = Math.max(...hrs, 150) + 5;
    const kMax = Math.max(1, ...K.map((v) => v ?? 0));
    const x = (v: number) => box.l + (v / xMax) * (box.r - box.l);
    const y = (v: number) => box.b - ((v - yLo) / (yHi - yLo)) * (box.b - box.t);
    drawAxes(ctx, theme, box, ticks(0, xMax, 5).map((v) => ({ v, x: x(v), label: String(v) })), ticks(yLo, yHi, 4).map((v) => ({ v, y: y(v), label: String(v) })));
    const dense = P.length > 5000;
    ctx.globalAlpha = dense ? 0.25 : 0.7;
    for (let i = 0; i < P.length; i += 1) {
      const p = P[i];
      const hr = H[i];
      if (p == null || hr == null) continue;
      const q = Math.min(4, Math.floor(((K[i] ?? 0) / kMax) * 5));
      ctx.fillStyle = theme.fatigue[q] ?? theme.muted;
      ctx.fillRect(x(p) - 1.5, y(hr) - 1.5, 3, 3);
    }
    ctx.globalAlpha = 1;
  }, [rec, theme]);
  const kMax = Math.max(1, ...(rec.series["kj"] ?? []).map((v) => v ?? 0));
  return (
    <section className="rv-card">
      <header className="rv-head"><h3>Power × HR by work done</h3></header>
      <div className="rv-canvas"><canvas ref={ref} className="fill-canvas" role="img" aria-label="power versus heart rate coloured by kJ" /></div>
      <div className="ramp-legend">
        {[0, 1, 2, 3, 4].map((i) => (
          <span key={i}><i style={{ background: `var(--fatigue-${String(i + 1)})` }} />{Math.round((kMax * i) / 5)}–{Math.round((kMax * (i + 1)) / 5)} kJ</span>
        ))}
      </div>
    </section>
  );
}

/** P12 per block — τ of each finished HR recovery vs the usual τ. */
export function TauChart({ recs }: { recs: HrRecoveryState[] }) {
  const theme = useChartTheme();
  const { ref } = useCanvas((ctx, w, h) => {
    const fits = recs.flatMap((r) => (r.fit ? [r.fit] : []));
    const box: Box = { l: 34, t: 10, r: w - 10, b: h - 18 };
    const usual = fits[0]?.usualTauS ?? 55;
    const yHi = Math.max(usual * 1.5, ...fits.map((f) => f.tauS)) * 1.1;
    const n = Math.max(1, fits.length);
    const x = (i: number) => box.l + ((i + 0.5) / n) * (box.r - box.l);
    const y = (v: number) => box.b - (v / yHi) * (box.b - box.t);
    drawAxes(ctx, theme, box, fits.map((_, i) => ({ v: i, x: x(i), label: `#${String(i + 1)}` })), ticks(0, yHi, 4).map((v) => ({ v, y: y(v), label: `${String(v)}s` })));
    ctx.strokeStyle = theme.target;
    ctx.setLineDash([6, 4]);
    ctx.beginPath();
    ctx.moveTo(box.l, y(usual));
    ctx.lineTo(box.r, y(usual));
    ctx.stroke();
    ctx.setLineDash([]);
    ctx.fillStyle = theme.ink2;
    ctx.font = `11px ${theme.font}`;
    ctx.textAlign = "right";
    ctx.fillText(`usual τ ${String(Math.round(usual))} s`, box.r, y(usual) - 4);
    fits.forEach((f, i) => {
      ctx.beginPath();
      ctx.arc(x(i), y(f.tauS), 5, 0, Math.PI * 2);
      ctx.fillStyle = theme.series.hr;
      ctx.strokeStyle = theme.surface1;
      ctx.lineWidth = 2;
      ctx.fill();
      ctx.stroke();
      ctx.fillStyle = theme.ink;
      ctx.textAlign = "center";
      ctx.fillText(`${String(Math.round(f.tauS))}s`, x(i), y(f.tauS) - 9);
    });
  }, [recs, theme]);
  return (
    <section className="rv-card">
      <header className="rv-head"><h3>HR recovery τ per block</h3></header>
      {recs.length ? (
        <div className="rv-canvas"><canvas ref={ref} className="fill-canvas" role="img" aria-label="recovery time constant per block" /></div>
      ) : (
        <p className="muted pad">No finished HR-guided recoveries yet.</p>
      )}
    </section>
  );
}
