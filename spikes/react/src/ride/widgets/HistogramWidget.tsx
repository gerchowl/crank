import { useState } from "react";
import { zoneIndex, type Histogram, type MetricId } from "@neo/shared";
import { backend } from "../../backend";
import { usePoll } from "../../lib/live";
import { useCanvas } from "../../charts/useCanvas";
import { drawAxes, ticks, type Box } from "../../charts/canvasAxes";
import { useChartTheme, zoneColor } from "../../theme/theme";
import { mmss } from "../../lib/format";
import { WidgetFrame, type WidgetProps } from "./common";
import { info, seriesColor } from "./metrics";
import { TableToggle, TableView } from "./TableView";

const BIN_CHOICES: Partial<Record<MetricId, number[]>> = { power: [5, 10, 25], hr: [2, 5, 10], cadence: [2, 5] };

// P06 — time-in-bin, zone-ramp fill, "typical" as an outline step line. Polled ≤ 0.5 Hz.
export function HistogramWidget({ widget, data: given }: WidgetProps & { data?: Histogram }) {
  const metric = widget.metrics[0] ?? "power";
  const m = info(metric);
  const choices = BIN_CHOICES[metric] ?? [1, 5];
  const [binWidth, setBinWidth] = useState(choices[1] ?? choices[0] ?? 5);
  const [polled, setPolled] = useState<Histogram | null>(null);
  const [table, setTable] = useState(false);
  const [hover, setHover] = useState<{ x: number; y: number; i: number } | null>(null);
  const theme = useChartTheme();
  usePoll(widget.rateHz, 0.5, () => {
    if (!given) setPolled(backend.histogram(metric, binWidth));
  }, binWidth);
  const data = given ?? polled;
  const zones = m.zones;
  const box = (w: number, h: number): Box => ({ l: 34, t: 6, r: w - 6, b: h - 18 });

  const { ref } = useCanvas((ctx, w, h) => {
    if (!data?.bins.length) return;
    const bx = box(w, h);
    const bins = data.bins;
    const lo = bins[0]?.lo ?? 0;
    const hi = (bins[bins.length - 1]?.lo ?? 0) + data.binWidth;
    const maxS = Math.max(1, ...bins.map((b) => b.seconds), ...(data.typical ?? []));
    const x = (v: number) => bx.l + ((v - lo) / (hi - lo)) * (bx.r - bx.l);
    const y = (s: number) => bx.b - (s / maxS) * (bx.b - bx.t);
    const yt = ticks(0, maxS / 60, 3).map((mn) => ({ v: mn, y: y(mn * 60), label: `${String(mn)}′` }));
    drawAxes(ctx, theme, bx, ticks(lo, hi, 5).map((v) => ({ v, x: x(v), label: String(v) })), yt);
    for (const b of bins) {
      const x0 = x(b.lo) + 1;
      const x1 = x(b.lo + data.binWidth) - 1;
      const top = y(b.seconds);
      ctx.fillStyle = widget.colorByZone && zones ? zoneColor(theme, zoneIndex(zones, b.lo + data.binWidth / 2), zones.length) : seriesColor(theme, metric);
      const r = Math.min(4, (x1 - x0) / 2, bx.b - top);
      ctx.beginPath();
      ctx.roundRect(x0, top, Math.max(0.5, x1 - x0), bx.b - top, [r, r, 0, 0]);
      ctx.fill();
    }
    if (data.typical) {
      ctx.strokeStyle = theme.ink2;
      ctx.lineWidth = 1.5;
      ctx.setLineDash([]);
      ctx.beginPath();
      data.typical.forEach((s, i) => {
        const b = bins[i];
        if (!b) return;
        const yy = y(s);
        if (i === 0) ctx.moveTo(x(b.lo), yy);
        else ctx.lineTo(x(b.lo), yy);
        ctx.lineTo(x(b.lo + data.binWidth), yy);
      });
      ctx.stroke();
    }
  }, [data, theme, widget.colorByZone]);

  const hovered = hover ? data?.bins[hover.i] : undefined;
  return (
    <WidgetFrame
      title={`${m.label} · time in bin`}
      className="hist"
      aside={
        <span className="seg">
          {choices.map((c) => (
            <button type="button" key={c} className={c === binWidth ? "on" : ""} onClick={() => { setBinWidth(c); }}>
              {c}
              {m.unit}
            </button>
          ))}
          <TableToggle on={table} onChange={setTable} />
        </span>
      }
    >
      {table ? (
        <TableView head={[`${m.label} (${m.unit})`, "time", "typical"]} rows={(data?.bins ?? []).filter((b) => b.seconds > 0).map((b, i) => [`${String(b.lo)}–${String(b.lo + (data?.binWidth ?? 0))}`, mmss(b.seconds), mmss(data?.typical?.[i] ?? NaN)])} />
      ) : (
        <div className="canvas-wrap">
          <canvas
            ref={ref}
            className="fill-canvas"
            role="img"
            aria-label={`${m.label} histogram`}
            onPointerMove={(e) => {
              const r = e.currentTarget.getBoundingClientRect();
              const bins = data?.bins;
              if (!bins?.length || !data) return;
              const bx = box(r.width, r.height);
              const lo = bins[0]?.lo ?? 0;
              const hi = (bins[bins.length - 1]?.lo ?? 0) + data.binWidth;
              const v = lo + ((e.clientX - r.left - bx.l) / (bx.r - bx.l)) * (hi - lo);
              const i = Math.floor((v - lo) / data.binWidth);
              setHover(i >= 0 && i < bins.length ? { x: e.clientX - r.left, y: e.clientY - r.top, i } : null);
            }}
            onPointerLeave={() => { setHover(null); }}
          />
          {hover && hovered && data && (
            <div className="tip" style={{ left: hover.x + 10, top: hover.y - 10 }}>
              {hovered.lo}–{hovered.lo + data.binWidth} {m.unit}: <b>{mmss(hovered.seconds)}</b>
              {data.typical?.[hover.i] !== undefined && <> · typical {mmss(data.typical[hover.i] ?? 0)}</>}
            </div>
          )}
        </div>
      )}
    </WidgetFrame>
  );
}
