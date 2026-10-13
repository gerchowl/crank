import { createMemo, createSignal, For, Show } from "solid-js";
import type { Histogram as Hist, MetricId } from "@neo/shared";
import { backend, metricInfo } from "../backend";
import { Canvas } from "../charts/Canvas";
import { clock, zoneIdx } from "../lib/format";
import { createPoll } from "../lib/primitives";
import { type ThemeTokens, zoneColor } from "../theme/theme";
import { seriesColor, type WidgetProps, zoneLabel, zonesOf } from "./common";
import { TableView } from "./TableView";

const BIN_CHOICES: Partial<Record<MetricId, number[]>> = {
  power: [5, 10, 25],
  power_3s: [5, 10, 25],
  hr: [2, 5, 10],
  cadence: [2, 5],
};

const PAD = { l: 34, r: 8, t: 8, b: 22 };

/** P06 — time-in-bin histogram, zone-ramp fill, "typical" as outline step line, bin-width picker. */
export function Histogram(props: WidgetProps & { data?: Hist }) {
  const metric = () => props.widget.metrics[0] ?? "power";
  const choices = () => BIN_CHOICES[metric()] ?? [1, 5, 10];
  const [bin, setBin] = createSignal<number | null>(null);
  const binWidth = () => bin() ?? choices()[1] ?? choices()[0] ?? 5;
  const [table, setTable] = createSignal(false);

  const polled = createPoll(
    () => backend.histogram(metric(), binWidth()),
    () => Math.min(props.widget.rateHz, 0.5),
    () => !props.data,
  );
  const hist = () => props.data ?? polled();

  const geom = createMemo((): Geom | null => {
    const h = hist();
    if (!h || h.bins.length === 0) return null;
    const lo = h.bins[0]?.lo ?? 0;
    const hi = (h.bins[h.bins.length - 1]?.lo ?? 0) + h.binWidth;
    const maxS = Math.max(1, ...h.bins.map((b) => b.seconds), ...(h.typical ?? []));
    return { h, lo, hi, maxS };
  });

  const draw = (ctx: CanvasRenderingContext2D, w: number, hgt: number, t: ThemeTokens) => {
    paint(ctx, w, hgt, t, geom(), metric(), props.widget.colorByZone === true);
  };

  const tooltip = (p: { x: number }, w: number) => {
    const g = geom();
    if (!g) return null;
    const v = g.lo + ((p.x - PAD.l) / (w - PAD.l - PAD.r)) * (g.hi - g.lo);
    const i = Math.floor((v - g.lo) / g.h.binWidth);
    const b = g.h.bins[i];
    if (!b) return null;
    const typ = g.h.typical?.[i];
    const unit = metricInfo(metric()).unit;
    return (
      <>
        <b>
          {Math.round(b.lo)}–{Math.round(b.lo + g.h.binWidth)} {unit}
        </b>
        <div>{clock(b.seconds)} in bin</div>
        <Show when={typ !== undefined}>
          <div class="muted">typical {clock(typ ?? 0)}</div>
        </Show>
      </>
    );
  };

  return (
    <div class="chart-widget">
      <div class="w-label">
        <span>
          {metricInfo(metric()).label} · time in bin <Show when={hist()?.typical}>
            <span class="muted">— typical</span>
          </Show>
        </span>
        <span class="w-tools">
          <For each={choices()}>
            {(c) => (
              <button class="mini" classList={{ on: binWidth() === c }} onClick={() => setBin(c)}>
                {c}
              </button>
            )}
          </For>
          <button class="mini" classList={{ on: table() }} onClick={() => setTable(!table())} title="table view">
            ▦
          </button>
        </span>
      </div>
      <Show
        when={!table()}
        fallback={
          <TableView
            caption={`${metricInfo(metric()).label} time in bin`}
            head={["bin", "time", "typical"]}
            rows={(hist()?.bins ?? [])
              .filter((b) => b.seconds > 0)
              .map((b) => [`${Math.round(b.lo)}–${Math.round(b.lo + (hist()?.binWidth ?? 0))}`, clock(b.seconds), clock(hist()?.typical?.[hist()?.bins.indexOf(b) ?? 0] ?? 0)])}
          />
        }
      >
        <Canvas draw={draw} tooltip={tooltip} ariaLabel={`${metricInfo(metric()).label} histogram`} />
      </Show>
    </div>
  );
}

interface Geom {
  h: Hist;
  lo: number;
  hi: number;
  maxS: number;
}

/** Pure canvas painter (no reactivity). */
function paint(ctx: CanvasRenderingContext2D, w: number, hgt: number, t: ThemeTokens, g: Geom | null, metric: MetricId, colorByZone: boolean) {
  ctx.font = `11px ${t.font}`;
  if (!g) {
    ctx.fillStyle = t.muted;
    ctx.fillText("no data yet", PAD.l, hgt / 2);
    return;
  }
  const zs = zonesOf(metric);
  const pw = w - PAD.l - PAD.r;
  const ph = hgt - PAD.t - PAD.b;
  const X = (v: number) => PAD.l + ((v - g.lo) / (g.hi - g.lo)) * pw;
  const Y = (s: number) => PAD.t + ph - (s / g.maxS) * ph;
  // recessive grid (minutes)
  ctx.strokeStyle = t.grid;
  ctx.fillStyle = t.muted;
  ctx.lineWidth = 1;
  const stepMin = Math.max(1, Math.ceil(g.maxS / 60 / 4));
  for (let m = 0; m * 60 <= g.maxS; m += stepMin) {
    const y = Math.round(Y(m * 60)) + 0.5;
    ctx.beginPath();
    ctx.moveTo(PAD.l, y);
    ctx.lineTo(w - PAD.r, y);
    ctx.stroke();
    ctx.fillText(`${m}m`, 4, y + 4);
  }
  // bars: 2 px surface gap, zone-ramp fill (series colour when the metric has no zones)
  const bw = Math.max(1, (g.h.binWidth / (g.hi - g.lo)) * pw);
  for (const b of g.h.bins) {
    if (!b.seconds) continue;
    const fill = colorByZone && zs ? zoneColor(t, zoneIdx(zs, b.lo + g.h.binWidth / 2)) : seriesColor(t, metric);
    ctx.fillStyle = fill;
    const x = X(b.lo) + 1;
    const y = Y(b.seconds);
    const width = Math.max(1, bw - 2);
    const r = Math.min(3, width / 2);
    ctx.beginPath();
    ctx.roundRect(x, y, width, PAD.t + ph - y, [r, r, 0, 0]);
    ctx.fill();
  }
  // typical: outline step line
  if (g.h.typical) {
    ctx.strokeStyle = t.ink2;
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    for (const [i, b] of g.h.bins.entries()) {
      const y = Y(g.h.typical[i] ?? 0);
      if (i === 0) ctx.moveTo(X(b.lo), y);
      else ctx.lineTo(X(b.lo), y);
      ctx.lineTo(X(b.lo + g.h.binWidth), y);
    }
    ctx.stroke();
  }
  // baseline + zone boundary labels on the axis
  ctx.strokeStyle = t.axis;
  ctx.beginPath();
  ctx.moveTo(PAD.l, PAD.t + ph + 0.5);
  ctx.lineTo(w - PAD.r, PAD.t + ph + 0.5);
  ctx.stroke();
  ctx.fillStyle = t.muted;
  if (zs) {
    for (const [i, z] of zs.entries()) {
      if (z.max <= g.lo || z.min >= g.hi) continue;
      const mid = (Math.max(z.min, g.lo) + Math.min(z.max, g.hi)) / 2;
      ctx.fillText(zoneLabel(z, i), X(mid) - 7, hgt - 6);
      if (z.min > g.lo) ctx.fillRect(Math.round(X(z.min)), PAD.t + ph, 1, 4);
    }
  } else {
    ctx.fillText(`${Math.round(g.lo)}`, PAD.l, hgt - 6);
    ctx.fillText(`${Math.round(g.hi)} ${metricInfo(metric).unit}`, w - PAD.r - 50, hgt - 6);
  }
}
