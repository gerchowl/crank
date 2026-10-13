import { createMemo, createSignal, onMount, Show } from "solid-js";
import type uPlot from "uplot";
import { backend, metricInfo } from "../backend";
import { UPlot } from "../charts/UPlot";
import { formatValue, zoneIdx } from "../lib/format";
import { createSample } from "../lib/primitives";
import { ride } from "../state/ride";
import { ZoneChip } from "./chips";
import { seriesColor, type WidgetProps, zoneLabel, zonesOf } from "./common";

/** P02 — number + short sparkline (ring buffer of this subscription's samples). */
export function NumberSparkline(props: WidgetProps) {
  const metric = () => props.widget.metrics[0] ?? "hr";
  const span = () => props.widget.spanS ?? 300;
  const xs: number[] = [];
  const ys: (number | null)[] = [];
  const [data, setData] = createSignal<uPlot.AlignedData>([[], []], { equals: false });

  const push = (t: number, v: number | null) => {
    const last = xs[xs.length - 1];
    if (last !== undefined && t < last) {
      xs.length = 0; // ride restarted
      ys.length = 0;
    }
    xs.push(t);
    ys.push(v);
    const cut = t - span();
    let drop = 0;
    while (drop < xs.length && (xs[drop] ?? 0) < cut) drop++;
    if (drop) {
      xs.splice(0, drop);
      ys.splice(0, drop);
    }
    setData([xs, ys]);
  };

  // backfill from the multi-resolution store so the line isn't empty after a page switch
  onMount(() => {
    const st = backend.history({ metric: metric(), spanS: span(), buckets: 90, scale: "fisheye" });
    const now = ride.status.t;
    for (const b of st) push(now - b.tAgo - b.dur / 2, b.mean);
  });

  const sample = createSample(
    () => ({ metric: metric(), rateHz: props.widget.rateHz, filter: props.widget.filter }),
    (s) => {
      if (s.t > 0) push(s.t, s.v);
    },
  );
  const value = () => sample()?.v ?? null;
  const zone = createMemo(() => {
    const zs = zonesOf(metric());
    const v = value();
    if (!props.widget.colorByZone || !zs || v === null) return null;
    const i = zoneIdx(zs, v);
    return { idx: i, label: zoneLabel(zs[i], i) };
  });

  return (
    <div class="spark">
      <div class="w-label">{metricInfo(metric()).label}</div>
      <div class="bignum-row">
        <span class="bignum-value small">{formatValue(metric(), value())}</span>
        <span class="bignum-unit">{metricInfo(metric()).unit}</span>
        <Show when={zone()}>{(z) => <ZoneChip idx={z().idx} label={z().label} />}</Show>
      </div>
      <UPlot
        class="spark-plot"
        data={data}
        options={(t) => ({
          legend: { show: false },
          cursor: { show: false },
          padding: [4, 2, 2, 2],
          scales: { x: { time: false } },
          axes: [{ show: false }, { show: false }],
          series: [{}, { stroke: seriesColor(t, metric()), width: 2, spanGaps: false, points: { show: false } }],
        })}
      />
    </div>
  );
}
