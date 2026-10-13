import { createMemo, createSignal, For, Show } from "solid-js";
import type { Heatmap2D, MetricId, Zone } from "@neo/shared";
import { backend, metricInfo } from "../backend";
import { Canvas, type Hover } from "../charts/Canvas";
import { clock } from "../lib/format";
import { createPoll } from "../lib/primitives";
import type { ThemeTokens } from "../theme/theme";
import { type WidgetProps, zoneLabel, zonesOf } from "./common";
import { TableView } from "./TableView";

const BIN: Partial<Record<MetricId, number>> = { power: 10, hr: 2, cadence: 2 };
const PAIRS: [MetricId, MetricId][] = [
  ["power", "hr"],
  ["power", "cadence"],
];
const PAD = { l: 34, r: 8, t: 8, b: 20 };

/** P07 — 2D heatmap, sequential fill = seconds, zone gridlines, x/y picker, cell hover. */
export function Heatmap(props: WidgetProps & { data?: Heatmap2D }) {
  const [pair, setPair] = createSignal<[MetricId, MetricId] | null>(null);
  const [table, setTable] = createSignal(false);
  const x = () => pair()?.[0] ?? props.widget.metrics[0] ?? "power";
  const y = () => pair()?.[1] ?? props.widget.metrics[1] ?? "hr";
  const polled = createPoll(
    () => backend.heatmap(x(), y(), BIN[x()] ?? 5, BIN[y()] ?? 2),
    () => Math.min(props.widget.rateHz, 0.5),
    () => !props.data,
  );
  const hm = () => props.data ?? polled();
  const maxS = createMemo(() => Math.max(1, ...(hm()?.seconds ?? [1])));

  const draw = (ctx: CanvasRenderingContext2D, w: number, h: number, t: ThemeTokens) => {
    paint(ctx, w, h, t, hm(), maxS(), zonesOf(x()), zonesOf(y()));
  };

  const tooltip = (p: Hover, w: number, h: number) => {
    const m = hm();
    if (!m || !m.cols) return null;
    const c = Math.floor(((p.x - PAD.l) / (w - PAD.l - PAD.r)) * m.cols);
    const r = Math.floor(((h - PAD.b - p.y) / (h - PAD.t - PAD.b)) * m.rows);
    if (c < 0 || r < 0 || c >= m.cols || r >= m.rows) return null;
    const s = m.seconds[r * m.cols + c] ?? 0;
    return (
      <>
        <b>
          {Math.round(m.xLo + c * m.xBin)}–{Math.round(m.xLo + (c + 1) * m.xBin)} {metricInfo(x()).unit} ×{" "}
          {Math.round(m.yLo + r * m.yBin)}–{Math.round(m.yLo + (r + 1) * m.yBin)} {metricInfo(y()).unit}
        </b>
        <div>{clock(s)}</div>
      </>
    );
  };

  const rows = createMemo(() => {
    const m = hm();
    if (!m) return [];
    const out: (string | number)[][] = [];
    for (let r = 0; r < m.rows; r++)
      for (let c = 0; c < m.cols; c++) {
        const s = m.seconds[r * m.cols + c] ?? 0;
        if (s > 0) out.push([Math.round(m.xLo + c * m.xBin), Math.round(m.yLo + r * m.yBin), clock(s)]);
      }
    return out.sort((a, b) => String(b[2]).localeCompare(String(a[2]))).slice(0, 200);
  });

  return (
    <div class="chart-widget">
      <div class="w-label">
        <span>
          {metricInfo(x()).label} × {metricInfo(y()).label}
        </span>
        <span class="w-tools">
          <For each={PAIRS}>
            {(pr) => (
              <button class="mini" classList={{ on: x() === pr[0] && y() === pr[1] }} onClick={() => setPair(pr)}>
                {pr[1] === "hr" ? "P×HR" : "P×cad"}
              </button>
            )}
          </For>
          <button class="mini" classList={{ on: table() }} onClick={() => setTable(!table())} title="table view">
            ▦
          </button>
        </span>
      </div>
      <Show when={!table()} fallback={<TableView caption="heatmap cells (top 200)" head={[metricInfo(x()).unit, metricInfo(y()).unit, "time"]} rows={rows()} />}>
        <Canvas draw={draw} tooltip={tooltip} ariaLabel={`${x()} by ${y()} heatmap`} />
      </Show>
    </div>
  );
}

function paint(ctx: CanvasRenderingContext2D, w: number, h: number, t: ThemeTokens, m: Heatmap2D | undefined, maxS: number, xz: Zone[] | undefined, yz: Zone[] | undefined) {
  ctx.font = `11px ${t.font}`;
  if (!m || m.cols === 0) {
    ctx.fillStyle = t.muted;
    ctx.fillText("no data yet", PAD.l, h / 2);
    return;
  }
  const pw = w - PAD.l - PAD.r;
  const ph = h - PAD.t - PAD.b;
  const cw = pw / m.cols;
  const ch = ph / m.rows;
  const xHi = m.xLo + m.cols * m.xBin;
  const yHi = m.yLo + m.rows * m.yBin;
  const X = (v: number) => PAD.l + ((v - m.xLo) / (xHi - m.xLo)) * pw;
  const Y = (v: number) => PAD.t + ph - ((v - m.yLo) / (yHi - m.yLo)) * ph;
  // sequential fill (sqrt scale so short dwell stays visible)
  for (let r = 0; r < m.rows; r++) {
    for (let c = 0; c < m.cols; c++) {
      const s = m.seconds[r * m.cols + c] ?? 0;
      if (!s) continue;
      const k = Math.min(6, Math.floor(Math.sqrt(s / maxS) * 7));
      ctx.fillStyle = t.seq[k] ?? t.accent;
      ctx.fillRect(PAD.l + c * cw, PAD.t + ph - (r + 1) * ch, Math.max(1, cw - 0.5), Math.max(1, ch - 0.5));
    }
  }
  // zone gridlines + labels
  ctx.strokeStyle = t.axis;
  ctx.fillStyle = t.muted;
  ctx.lineWidth = 1;
  ctx.setLineDash([2, 3]);
  for (const [i, z] of (xz ?? []).entries()) {
    if (z.min <= m.xLo || z.min >= xHi) continue;
    const px = Math.round(X(z.min)) + 0.5;
    ctx.beginPath();
    ctx.moveTo(px, PAD.t);
    ctx.lineTo(px, PAD.t + ph);
    ctx.stroke();
    ctx.fillText(zoneLabel(z, i), px + 2, h - 6);
  }
  for (const [i, z] of (yz ?? []).entries()) {
    if (z.min <= m.yLo || z.min >= yHi) continue;
    const py = Math.round(Y(z.min)) + 0.5;
    ctx.beginPath();
    ctx.moveTo(PAD.l, py);
    ctx.lineTo(PAD.l + pw, py);
    ctx.stroke();
    ctx.fillText(zoneLabel(z, i), 4, py + 4);
  }
  ctx.setLineDash([]);
  if (!yz) {
    ctx.fillText(String(Math.round(m.yLo)), 4, PAD.t + ph);
    ctx.fillText(String(Math.round(yHi)), 4, PAD.t + 10);
  }
}
