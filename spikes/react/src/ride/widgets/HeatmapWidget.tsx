import { useState } from "react";
import type { Heatmap2D, MetricId } from "@neo/shared";
import { backend } from "../../backend";
import { usePoll } from "../../lib/live";
import { useCanvas } from "../../charts/useCanvas";
import { drawAxes, rampAt, ticks, type Box } from "../../charts/canvasAxes";
import { useChartTheme } from "../../theme/theme";
import { mmss } from "../../lib/format";
import { WidgetFrame, type WidgetProps } from "./common";
import { info } from "./metrics";
import { TableToggle, TableView } from "./TableView";

const BIN: Partial<Record<MetricId, number>> = { power: 20, hr: 4, cadence: 4 };
const Y_CHOICES: MetricId[] = ["hr", "cadence"];

// P07 — 2D heatmap, sequential fill = seconds, power-zone gridlines; cell hover; y picker.
export function HeatmapWidget({ widget, data: given }: WidgetProps & { data?: Heatmap2D }) {
  const xM = widget.metrics[0] ?? "power";
  const [yM, setYM] = useState<MetricId>(widget.metrics[1] ?? "hr");
  const [polled, setPolled] = useState<Heatmap2D | null>(null);
  const [table, setTable] = useState(false);
  const [hover, setHover] = useState<{ x: number; y: number; c: number; r: number } | null>(null);
  const theme = useChartTheme();
  usePoll(widget.rateHz, 0.5, () => {
    if (!given) setPolled(backend.heatmap(xM, yM, BIN[xM] ?? 10, BIN[yM] ?? 4));
  }, yM);
  const data = given ?? polled;
  const box = (w: number, h: number): Box => ({ l: 34, t: 6, r: w - 6, b: h - 18 });

  const { ref } = useCanvas((ctx, w, h) => {
    if (!data || !data.cols || !data.rows) return;
    const bx = box(w, h);
    const cw = (bx.r - bx.l) / data.cols;
    const ch = (bx.b - bx.t) / data.rows;
    const max = Math.max(1, ...data.seconds);
    const xHi = data.xLo + data.cols * data.xBin;
    const yHi = data.yLo + data.rows * data.yBin;
    const xOf = (v: number) => bx.l + ((v - data.xLo) / (xHi - data.xLo)) * (bx.r - bx.l);
    const yOf = (v: number) => bx.b - ((v - data.yLo) / (yHi - data.yLo)) * (bx.b - bx.t);
    drawAxes(ctx, theme, bx, ticks(data.xLo, xHi, 5).map((v) => ({ v, x: xOf(v), label: String(v) })), ticks(data.yLo, yHi, 4).map((v) => ({ v, y: yOf(v), label: String(v) })));
    for (let r = 0; r < data.rows; r++) {
      for (let c = 0; c < data.cols; c++) {
        const s = data.seconds[r * data.cols + c] ?? 0;
        if (s <= 0) continue;
        ctx.fillStyle = rampAt(theme.seq, Math.sqrt(s / max));
        ctx.fillRect(bx.l + c * cw + 0.5, bx.b - (r + 1) * ch + 0.5, Math.max(0.5, cw - 1), Math.max(0.5, ch - 1));
      }
    }
    if (xM.startsWith("power")) {
      ctx.strokeStyle = theme.muted;
      ctx.setLineDash([3, 3]);
      ctx.lineWidth = 1;
      ctx.font = `10px ${theme.font}`;
      ctx.fillStyle = theme.muted;
      ctx.textAlign = "left";
      ctx.textBaseline = "top";
      backend.zones.power.forEach((z, i) => {
        const xx = xOf(z.min);
        if (xx <= bx.l || xx >= bx.r) return;
        ctx.beginPath();
        ctx.moveTo(xx, bx.t);
        ctx.lineTo(xx, bx.b);
        ctx.stroke();
        ctx.fillText(`Z${String(i + 1)}`, xx + 2, bx.t);
      });
      ctx.setLineDash([]);
    }
  }, [data, theme, xM]);

  const cellS = hover && data ? (data.seconds[hover.r * data.cols + hover.c] ?? 0) : 0;
  return (
    <WidgetFrame
      title={`${info(xM).label} × ${info(yM).label}`}
      className="heat"
      aside={
        <span className="seg">
          {Y_CHOICES.map((c) => (
            <button type="button" key={c} className={c === yM ? "on" : ""} onClick={() => { setYM(c); }}>
              {info(c).unit}
            </button>
          ))}
          <TableToggle on={table} onChange={setTable} />
        </span>
      }
    >
      {table && data ? (
        <TableView
          head={[info(xM).unit, info(yM).unit, "time"]}
          rows={data.seconds.flatMap((s, i) => (s > 0 ? [[data.xLo + (i % data.cols) * data.xBin, data.yLo + Math.floor(i / data.cols) * data.yBin, mmss(s)]] : []))}
        />
      ) : (
        <div className="canvas-wrap">
          <canvas
            ref={ref}
            className="fill-canvas"
            role="img"
            aria-label="heatmap"
            onPointerMove={(e) => {
              if (!data) return;
              const rc = e.currentTarget.getBoundingClientRect();
              const bx = box(rc.width, rc.height);
              const c = Math.floor(((e.clientX - rc.left - bx.l) / (bx.r - bx.l)) * data.cols);
              const r = Math.floor(((bx.b - (e.clientY - rc.top)) / (bx.b - bx.t)) * data.rows);
              setHover(c >= 0 && c < data.cols && r >= 0 && r < data.rows ? { x: e.clientX - rc.left, y: e.clientY - rc.top, c, r } : null);
            }}
            onPointerLeave={() => { setHover(null); }}
          />
          {hover && data && (
            <div className="tip" style={{ left: hover.x + 10, top: hover.y - 10 }}>
              {data.xLo + hover.c * data.xBin}–{data.xLo + (hover.c + 1) * data.xBin} {info(xM).unit} · {data.yLo + hover.r * data.yBin}–{data.yLo + (hover.r + 1) * data.yBin} {info(yM).unit}: <b>{mmss(cellS)}</b>
            </div>
          )}
        </div>
      )}
    </WidgetFrame>
  );
}
