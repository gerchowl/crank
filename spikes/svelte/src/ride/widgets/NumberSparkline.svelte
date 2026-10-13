<script lang="ts">
  import type { Sample, Widget } from "@neo/shared";
  import type uPlot from "uplot";
  import { liveMetric } from "../../lib/live.svelte";
  import { fmtValue, label, unit, zoneOf } from "../../lib/format";
  import { theme, seriesColor, zoneColor } from "../../lib/theme.svelte";
  import { uplot } from "../../charts/attach.svelte";

  let { widget }: { widget: Widget } = $props();

  const metric = $derived(widget.metrics[0]);
  const span = $derived(widget.spanS ?? 300);

  // Streaming buffer: plain arrays, never reactive. Samples are pushed straight into uPlot,
  // so a 1 Hz sparkline costs one setData per sample and zero component re-renders.
  let xs: number[] = [];
  let ys: (number | null)[] = [];
  let chart: uPlot | null = null;

  function push(s: Sample): void {
    if (xs.length && s.t < (xs.at(-1) ?? 0)) {
      xs = [];
      ys = [];
    }
    xs.push(s.t);
    ys.push(s.v);
    const cut = s.t - span;
    let i = 0;
    while (i < xs.length && (xs[i] ?? 0) < cut) i++;
    if (i) {
      xs.splice(0, i);
      ys.splice(0, i);
    }
    chart?.setData([xs, ys]);
  }

  const value = liveMetric(
    () => metric,
    () => (widget.filter ? { rateHz: widget.rateHz, filter: widget.filter } : { rateHz: widget.rateHz }),
    push,
  );
  const zone = $derived(widget.colorByZone && metric ? zoneOf(metric, value.v) : null);

  const options = (): Omit<uPlot.Options, "width" | "height"> => {
    const c = theme.colors;
    const m = metric ?? "power";
    return {
      legend: { show: false },
      cursor: { show: false },
      axes: [{ show: false }, { show: false }],
      scales: { x: { time: false }, y: { auto: true } },
      series: [{}, { stroke: seriesColor(c, m), width: 2, spanGaps: false, points: { show: false } }],
      padding: [4, 2, 2, 2],
    };
  };
</script>

<div class="tile">
  <div class="head">
    <span class="label">{metric ? label(metric) : "—"}</span>
    {#if zone}
      <span class="zone"><i style:background={zoneColor(theme.colors, zone.index, zone.n)}></i>{zone.name}</span>
    {/if}
  </div>
  <div class="row">
    <span class="value tabular">{metric ? fmtValue(metric, value.v) : "—"}<small>{metric ? unit(metric) : ""}</small></span>
    <div
      class="spark"
      {@attach uplot(options, () => [xs, ys], (u) => {
        chart = u;
      })}
    ></div>
  </div>
  <div class="foot muted">last {Math.round(span / 60)} min</div>
</div>

<style>
  .tile {
    display: flex;
    flex-direction: column;
    height: 100%;
    padding: 0.6rem 0.8rem;
    container-type: size;
  }
  .head {
    display: flex;
    gap: 0.5rem;
    font-size: 0.8rem;
    letter-spacing: 0.02em;
    font-weight: 600;
    color: var(--ink-secondary);
  }
  .label {
    flex: 1;
  }
  .zone {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    font-weight: 600;
    color: var(--ink-primary);
  }
  .zone i {
    width: 0.8rem;
    height: 0.8rem;
    border-radius: 3px;
  }
  .row {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 0.75rem;
    min-height: 0;
  }
  .value {
    font-weight: 650;
    font-size: clamp(1.8rem, min(45cqh, 18cqw), 7rem);
    line-height: 1;
    white-space: nowrap;
  }
  @container (min-height: 140px) {
    .value {
      font-size: clamp(64px, min(45cqh, 18cqw), 7rem);
    }
  }
  .value small {
    font-size: 0.3em;
    font-weight: 500;
    color: var(--ink-secondary);
    margin-left: 0.2em;
  }
  .spark {
    flex: 1;
    height: 70%;
    min-width: 60px;
    overflow: hidden;
  }
  .foot {
    font-size: 0.7rem;
  }
</style>
