import { useEffect, useRef } from "react";
import { backend } from "../../backend";
import { getStatus, useMetricCallback } from "../../lib/live";
import { fmtValue } from "../../lib/format";
import { useCanvas } from "../../charts/useCanvas";
import { useChartTheme } from "../../theme/theme";
import { WidgetFrame, type WidgetProps } from "./common";
import { info, seriesColor, zoneOf } from "./metrics";
import { zoneColor } from "../../theme/theme";

// P02 — fully imperative: the number and zone chip are written to the DOM, the line to a canvas.
export function NumberSparkline({ widget }: WidgetProps) {
  const m = info(widget.metrics[0] ?? "hr");
  const spanS = widget.spanS ?? 300;
  const theme = useChartTheme();
  const buf = useRef<{ t: number; v: number | null }[]>([]);
  const valueEl = useRef<HTMLSpanElement>(null);
  const chipEl = useRef<HTMLSpanElement>(null);
  const chipDot = useRef<HTMLElement>(null);
  const chipLabel = useRef<HTMLSpanElement>(null);

  const { ref, redraw } = useCanvas((ctx, w, h) => {
    const pts = buf.current;
    const last = pts[pts.length - 1];
    if (!last) return;
    let lo = Infinity;
    let hi = -Infinity;
    for (const p of pts) {
      if (p.v === null) continue;
      lo = Math.min(lo, p.v);
      hi = Math.max(hi, p.v);
    }
    if (!Number.isFinite(lo)) return;
    const pad = Math.max(1e-6, (hi - lo) * 0.15 || Math.abs(hi) * 0.05 || 1);
    lo -= pad;
    hi += pad;
    const x = (t: number) => w - ((last.t - t) / spanS) * w;
    const y = (v: number) => h - 2 - ((v - lo) / (hi - lo)) * (h - 4);
    ctx.lineWidth = 2;
    ctx.lineJoin = "round";
    ctx.strokeStyle = seriesColor(theme, m.id);
    ctx.beginPath();
    let pen = false;
    for (const p of pts) {
      if (p.v === null) {
        pen = false;
        continue;
      }
      if (pen) ctx.lineTo(x(p.t), y(p.v));
      else ctx.moveTo(x(p.t), y(p.v));
      pen = true;
    }
    ctx.stroke();
  }, theme);

  // Seed from the multi-resolution history so a remounted widget (page switch) isn't empty.
  useEffect(() => {
    const now = getStatus()?.t ?? 0;
    const hist = backend.history({ metric: m.id, spanS, buckets: 120, scale: "fisheye" });
    buf.current = hist.map((b) => ({ t: now - b.tAgo - b.dur / 2, v: b.mean }));
    redraw();
  }, [m.id, spanS, redraw]);

  // The line gets its own subscription at ≥ 4 Hz so it stays dense at 10×/60×;
  // the number updates at the widget's display rate.
  useMetricCallback(m.id, Math.max(4, widget.rateHz), widget.filter, (s) => {
    const pts = buf.current;
    const prev = pts[pts.length - 1];
    if (prev && s.t < prev.t - 5) pts.length = 0; // workout reloaded
    if (prev && s.t <= prev.t) return;
    pts.push(s);
    while (pts.length > 2 && (pts[0]?.t ?? 0) < s.t - spanS) pts.shift();
    redraw();
  });
  useMetricCallback(m.id, widget.rateHz, widget.filter, (s) => {
    if (valueEl.current) valueEl.current.textContent = fmtValue(s.v, m.unit, m.id);
    if (chipEl.current) {
      const hit = widget.colorByZone ? zoneOf(m.id, s.v) : null;
      chipEl.current.hidden = !hit;
      if (hit && chipLabel.current && chipDot.current) {
        chipLabel.current.textContent = hit.name.split(" ")[0] ?? "";
        chipDot.current.style.background = zoneColor(theme, hit.idx, hit.count);
      }
    }
  });

  return (
    <WidgetFrame title={m.label} className="sparkline">
      <div className="spark-top">
        <span className="mid-value" ref={valueEl}>—</span>
        <span className="unit">{m.unit}</span>
        <span className="zone-chip" ref={chipEl} hidden>
          <i ref={chipDot} />
          <span ref={chipLabel} />
        </span>
      </div>
      <canvas ref={ref} className="spark-canvas" aria-label={`${m.label} last ${Math.round(spanS / 60)} min`} />
    </WidgetFrame>
  );
}
