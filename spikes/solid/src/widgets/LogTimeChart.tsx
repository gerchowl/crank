import { createMemo, createSignal, For, Show } from "solid-js";
import type uPlot from "uplot";
import { type Bucket, type MetricId, timeAgoTicks, timeAgoToU, type TimeScale } from "@neo/shared";
import { backend, metricInfo } from "../backend";
import { axis, tooltipPlugin, UPlot } from "../charts/UPlot";
import { fmtTimeAgo } from "../lib/format";
import { uToTimeAgo } from "@neo/shared";
import { createPoll } from "../lib/primitives";
import { ride } from "../state/ride";
import type { ThemeTokens } from "../theme/theme";
import { withAlpha } from "../theme/theme";
import { seriesColor, type WidgetProps } from "./common";
import { TableView } from "./TableView";

const SPANS = [600, 1800, 3600, 7200];
let plotSeq = 0;
const BUCKETS = 160;

interface Panel {
  metric: MetricId;
  overlay?: MetricId; // target dashed in the power panel
}

/** Buckets → [x, mean, min, max] with x = 1 − u (now at the right) and null gaps. */
function toSeries(bs: Bucket[], span: number, scale: TimeScale): [number[], (number | null)[], (number | null)[], (number | null)[]] {
  const x: number[] = [];
  const mean: (number | null)[] = [];
  const lo: (number | null)[] = [];
  const hi: (number | null)[] = [];
  let prev: Bucket | undefined;
  for (const b of bs) {
    if (prev && Math.abs(prev.tAgo - (b.tAgo + b.dur)) > 1e-6) {
      x.push(1 - timeAgoToU(b.tAgo + b.dur, span, scale));
      mean.push(null);
      lo.push(null);
      hi.push(null);
    }
    x.push(1 - timeAgoToU(b.tAgo + b.dur / 2, span, scale));
    mean.push(b.mean);
    lo.push(b.min);
    hi.push(b.max);
    prev = b;
  }
  return [x, mean, lo, hi];
}

/** Sample overlay values at the panel's x positions (nearest bucket). */
function overlayAt(xs: number[], bs: Bucket[], span: number, scale: TimeScale): (number | null)[] {
  const ox = bs.map((b) => 1 - timeAgoToU(b.tAgo + b.dur / 2, span, scale));
  let j = 0;
  return xs.map((x) => {
    while (j < ox.length - 1 && Math.abs((ox[j + 1] ?? 0) - x) <= Math.abs((ox[j] ?? 0) - x)) j++;
    return bs[j]?.mean ?? null;
  });
}

/** P04 — log-time stacked panels sharing one x (time-ago, now at right); one crosshair. */
export function LogTimeChart(props: WidgetProps) {
  const [spanOverride, setSpan] = createSignal<number | null>(null);
  const [scaleOverride, setScale] = createSignal<TimeScale | null>(null);
  const [table, setTable] = createSignal(false);
  const span = () => spanOverride() ?? props.widget.spanS ?? 7200;
  const scale = (): TimeScale => scaleOverride() ?? props.widget.scale ?? "log";
  const syncKey = `ltc-${++plotSeq}`;

  const panels = createMemo((): Panel[] => {
    const ms = props.widget.metrics;
    const hasTarget = ms.includes("target_power");
    return ms
      .filter((m) => m !== "target_power")
      .map((m) => (m.startsWith("power") && hasTarget ? { metric: m, overlay: "target_power" as MetricId } : { metric: m }));
  });

  // one poll for all panels (≤ 1 Hz, polling budget), paused when the ride isn't moving
  const histories = createPoll(
    () => {
      const q = (metric: MetricId) => backend.history({ metric, spanS: span(), buckets: BUCKETS, scale: scale() });
      const out = new Map<MetricId, Bucket[]>();
      for (const p of panels()) {
        out.set(p.metric, q(p.metric));
        if (p.overlay) out.set(p.overlay, q(p.overlay));
      }
      return out;
    },
    () => Math.min(props.widget.rateHz, 1),
    () => ride.status.state === "riding" || ride.status.t > 0,
  );

  const ticks = createMemo(() => timeAgoTicks(span()).map((t) => ({ t, x: 1 - timeAgoToU(t, span(), scale()) })));

  const dataFor = (p: Panel) => (): uPlot.AlignedData => {
    const h = histories();
    const bs = h?.get(p.metric) ?? [];
    const [x, mean, lo, hi] = toSeries(bs, span(), scale());
    const overlay = p.overlay ? overlayAt(x, h?.get(p.overlay) ?? [], span(), scale()) : [];
    return p.overlay ? [x, lo, hi, mean, overlay] : [x, lo, hi, mean];
  };

  const options = (p: Panel, last: boolean) => (t: ThemeTokens) => {
    const color = seriesColor(t, p.metric);
    const series: uPlot.Series[] = [
      {},
      { label: "min", stroke: "transparent", points: { show: false } },
      { label: "max", stroke: "transparent", points: { show: false } },
      { label: metricInfo(p.metric).label, stroke: color, width: 2, points: { show: false } },
    ];
    if (p.overlay) series.push({ label: "target", stroke: t.target, width: 1.5, dash: [6, 4], points: { show: false } });
    const tk = ticks();
    const sp = span();
    const sc = scale();
    return {
      legend: { show: false },
      plugins: [tooltipPlugin((x) => `${fmtTimeAgo(Math.round(uToTimeAgo(1 - x, sp, sc)))} ago`, (v) => `${Math.round(v)} ${metricInfo(p.metric).unit}`, [3, 4])],
      cursor: { sync: { key: syncKey }, points: { size: 6 }, y: false, drag: { x: false, y: false } },
      padding: [6, 8, last ? 0 : 2, 0],
      bands: [{ series: [2, 1], fill: withAlpha(color, 0.18) }],
      scales: { x: { time: false, range: [0, 1] } },
      axes: [
        axis(t, {
          show: true,
          size: last ? 22 : 2,
          splits: () => tk.map((k) => k.x),
          values: () => (last ? tk.map((k) => fmtTimeAgo(k.t)) : tk.map(() => "")),
        }),
        axis(t, { size: 40, ticks: { show: false } }),
      ],
      series,
    } satisfies Omit<uPlot.Options, "width" | "height">;
  };

  const tableRows = createMemo(() => {
    const p = panels()[0];
    const bs = p ? (histories()?.get(p.metric) ?? []) : [];
    return bs
      .slice()
      .reverse()
      .filter((_, i) => i % 4 === 0)
      .map((b) => [fmtTimeAgo(Math.round(b.tAgo)), Math.round(b.mean), Math.round(b.min), Math.round(b.max)]);
  });

  return (
    <div class="ltc">
      <div class="w-label">
        <span>
          {panels()
            .map((p) => metricInfo(p.metric).label)
            .join(" · ")}{" "}
          <span class="muted">({scale() === "log" ? "log" : "fisheye"} time-ago)</span>
        </span>
        <span class="w-tools">
          <For each={SPANS}>
            {(s) => (
              <button class="mini" classList={{ on: span() === s }} onClick={() => setSpan(s)}>
                {fmtTimeAgo(s)}
              </button>
            )}
          </For>
          <button class="mini" onClick={() => setScale(scale() === "log" ? "fisheye" : "log")} title="log / fisheye">
            {scale() === "log" ? "log" : "fish"}
          </button>
          <button class="mini" classList={{ on: table() }} onClick={() => setTable(!table())} title="table view">
            ▦
          </button>
        </span>
      </div>
      <Show
        when={!table()}
        fallback={<TableView head={[`time-ago`, "mean", "min", "max"]} rows={tableRows()} caption={panels()[0]?.metric ?? ""} />}
      >
        <div class="ltc-panels">
          <For each={panels()}>
            {(p, i) => (
              <div class="ltc-panel">
                <div class="ltc-legend">
                  <span class="chip-line" style={{ background: `var(--series-${p.metric.startsWith("power") ? "power" : p.metric === "hr" ? "hr" : "cadence"})` }} />
                  {metricInfo(p.metric).unit}
                  <Show when={p.overlay}>
                    <span class="chip-dash" /> target
                  </Show>
                </div>
                <UPlot data={dataFor(p)} options={options(p, i() === panels().length - 1)} />
              </div>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}
