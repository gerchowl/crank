<script lang="ts">
  import type { MetricId, TimeScale, Widget } from "@neo/shared";
  import uPlot from "uplot";
  import { backend } from "../../lib/backend";
  import { poll } from "../../lib/live.svelte";
  import { ride } from "../../lib/ride.svelte";
  import { theme, seriesColor, withAlpha } from "../../lib/theme.svelte";
  import { label, unit, fmtValue } from "../../lib/format";
  import { uplot } from "../../charts/attach.svelte";
  import { axisStyle, bucketsToSeries, timeAgoAxis, alignTo } from "../../charts/style";
  import { uToTimeAgo } from "@neo/shared";
  import { dur } from "../../lib/format";

  // P04: log-time stacked panels. One panel per non-target metric (no dual axes);
  // target_power is a neutral dashed overlay in the power panel. Panels share x and a
  // synced crosshair. history() polled at ≤ 1 Hz, uPlot updated in place.
  let { widget, compact = false }: { widget: Widget; compact?: boolean } = $props();

  // Writable deriveds: follow the widget config, but the span/scale buttons can override
  // locally until the config changes again.
  let scale = $derived<TimeScale>(widget.scale ?? "log");
  let spanS = $derived(widget.spanS ?? 7200);
  const SPANS = [600, 1800, 3600, 7200];

  const panels = $derived(widget.metrics.filter((m) => m !== "target_power" && m !== "target_cadence"));
  const hasTarget = $derived(widget.metrics.includes("target_power"));
  const BUCKETS = 240;

  type PanelData = [number[], (number | null)[], (number | null)[], (number | null)[], (number | null)[]];
  const data = poll(
    () => {
      // eslint-disable-next-line svelte/prefer-svelte-reactivity -- immutable poll result, replaced wholesale ($state.raw)
      const out = new Map<MetricId, PanelData>();
      const s = spanS;
      const sc = scale;
      const target = hasTarget ? bucketsToSeries(backend.history({ metric: "target_power", spanS: s, buckets: BUCKETS, scale: sc }), s, sc) : null;
      for (const m of panels) {
        const [x, mean, mn, mx] = bucketsToSeries(backend.history({ metric: m, spanS: s, buckets: BUCKETS, scale: sc }), s, sc);
        const tgt = m.startsWith("power") && target ? alignTo(x, target[0], target[1]) : x.map(() => null);
        out.set(m, [x, mean, mn, mx, tgt]);
      }
      return out;
    },
    () => Math.min(1, widget.rateHz),
    () => ride.state === "riding",
  );

  const syncKey = `ltc-${Math.random().toString(36).slice(2)}`;
  let hover = $state<{ u: number; values: { m: MetricId; v: number | null }[] } | null>(null);

  function options(m: MetricId, last: boolean): () => Omit<uPlot.Options, "width" | "height"> {
    return () => {
      const c = theme.colors;
      const col = seriesColor(c, m);
      return {
        legend: { show: false },
        cursor: {
          sync: { key: syncKey },
          y: false,
          points: { size: 8, stroke: c.surface, width: 2 },
        },
        scales: { x: { time: false, range: [0, 1] }, y: { auto: true, range: (_u, lo, hi) => [Math.max(0, Math.floor(lo * 0.9)), Math.ceil(hi * 1.05 + 1)] } },
        axes: [timeAgoAxis(c, spanS, scale, last), axisStyle(c, { size: 40, values: (_u, v) => v.map((x) => `${x}`) })],
        series: [
          {},
          { stroke: col, width: 2, points: { show: false } },
          { stroke: "transparent", width: 0, points: { show: false } },
          { stroke: "transparent", width: 0, points: { show: false } },
          { stroke: c.target, width: 1.5, dash: [6, 4], points: { show: false } },
        ],
        bands: [{ series: [3, 2], fill: withAlpha(col.startsWith("#") ? col : "#888888", 0.22) }],
        padding: [6, 8, 0, 0],
        hooks: {
          setCursor: [
            (u) => {
              const idx = u.cursor.idx;
              if (idx === null || idx === undefined || u.cursor.left === undefined || u.cursor.left < 0) {
                hover = null;
                return;
              }
              const x = u.data[0][idx];
              if (x === undefined) return;
              const d = data.value;
              hover = {
                u: x,
                values: panels.map((pm) => {
                  const pd = d?.get(pm);
                  if (!pd) return { m: pm, v: null };
                  let j = 0;
                  while (j + 1 < pd[0].length && (pd[0][j + 1] ?? 2) <= x) j++;
                  return { m: pm, v: pd[1][j] ?? null };
                }),
              };
            },
          ],
        },
      };
    };
  }

  const empty: uPlot.AlignedData = [[], [], [], [], []];
</script>

<div class="wrap">
  <div class="head">
    <div class="legend">
      {#each panels as m (m)}
        <span><i style:background={seriesColor(theme.colors, m)}></i>{label(m)} <span class="muted">{unit(m)}</span></span>
      {/each}
      {#if hasTarget}<span><i class="dash"></i>target</span>{/if}
      <span class="muted">mean + min/max</span>
    </div>
    {#if !compact}
      <div class="ctl">
        {#each SPANS as s (s)}
          <button type="button" class="ghost" aria-pressed={spanS === s} onclick={() => (spanS = s)}>{dur(s)}</button>
        {/each}
        <button type="button" class="ghost" aria-pressed={scale === "fisheye"} onclick={() => (scale = scale === "log" ? "fisheye" : "log")} title="Toggle log / fisheye time axis">{scale}</button>
      </div>
    {/if}
  </div>
  <div class="panels">
    {#each panels as m, i (m)}
      <div class="panel" {@attach uplot(options(m, i === panels.length - 1), () => (data.value?.get(m) ?? empty))}></div>
    {/each}
    {#if hover}
      <div class="tip">
        <b>{dur(uToTimeAgo(1 - hover.u, spanS, scale))} ago</b>
        {#each hover.values as hv (hv.m)}
          <span>{label(hv.m)} {fmtValue(hv.m, hv.v)} {unit(hv.m)}</span>
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  .wrap {
    height: 100%;
    display: flex;
    flex-direction: column;
    padding: 0.4rem 0.6rem 0.2rem;
  }
  .head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.75rem;
  }
  .legend {
    display: flex;
    gap: 0.8rem;
    color: var(--ink-secondary);
    flex-wrap: wrap;
  }
  .legend i {
    display: inline-block;
    width: 12px;
    height: 3px;
    border-radius: 2px;
    margin-right: 4px;
    vertical-align: middle;
  }
  .legend i.dash {
    background: repeating-linear-gradient(90deg, var(--target) 0 4px, transparent 4px 7px);
  }
  .ctl {
    display: flex;
    gap: 2px;
  }
  .ctl button {
    min-height: 24px;
    padding: 0 0.4rem;
    font-size: 0.72rem;
  }
  .panels {
    position: relative;
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .panel {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
  .panel:last-of-type {
    flex: 1.25;
  }
  .tip {
    position: absolute;
    top: 4px;
    left: 48px;
    display: flex;
    gap: 0.75rem;
    font-size: 0.75rem;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0.15rem 0.5rem;
    pointer-events: none;
    font-variant-numeric: tabular-nums;
  }
</style>
