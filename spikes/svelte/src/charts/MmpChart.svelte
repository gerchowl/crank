<script lang="ts">
  import type { MmpCurve } from "@neo/shared";
  import type uPlot from "uplot";
  import { uplot } from "./attach.svelte";
  import { axisStyle } from "./style";
  import { theme } from "../lib/theme.svelte";
  import { dur } from "../lib/format";
  import ChartFrame from "./ChartFrame.svelte";

  // P09 power-duration (MMP) curve, log x 1 s–60 min. Three scopes of the same metric:
  // today = power series colour (identity), 90 d and all-time = secondary/muted ink, dashed
  // for all-time. Legend + direct labels; crosshair tooltip shows % of best.
  let { curve, title = "Mean-maximal power" }: { curve: MmpCurve | undefined; title?: string } = $props();

  let hover = $state<{ i: number; left: number } | null>(null);

  const data = $derived.by((): uPlot.AlignedData => {
    const c = curve;
    if (!c) return [[], [], [], []];
    const today = c.today.map((v) => (v > 0 ? v : null));
    return [c.durationsS, today, c.best90d, c.allTime];
  });

  const options = (): Omit<uPlot.Options, "width" | "height"> => {
    const c = theme.colors;
    return {
      legend: { show: false },
      cursor: { y: false, points: { size: 8, stroke: c.surface, width: 2 } },
      // eslint-disable-next-line @typescript-eslint/no-unsafe-enum-assignment -- uPlot.Scale.Distr is an ambient const enum (unusable under isolatedModules); 3 = Logarithmic
      scales: { x: { time: false, distr: 3, log: 10 }, y: { auto: true, range: (_u, _lo, hi) => [0, Math.ceil((hi * 1.08) / 50) * 50] } },
      axes: [
        axisStyle(c, { size: 22, splits: () => [1, 5, 15, 60, 300, 1200, 3600], values: (_u, s) => s.map((x) => (x === null ? "" : dur(x))) }),
        axisStyle(c, { size: 38 }),
      ],
      series: [
        {},
        { stroke: c.power, width: 2.5, points: { show: false } },
        { stroke: c.inkSecondary, width: 1.5, points: { show: false } },
        { stroke: c.muted, width: 1.5, dash: [5, 4], points: { show: false } },
      ],
      padding: [8, 34, 0, 0],
      hooks: {
        setCursor: [
          (u) => {
            const i = u.cursor.idx;
            hover = i === null || i === undefined ? null : { i, left: u.cursor.left ?? 0 };
          },
        ],
        draw: [
          (u) => {
            // direct labels at the right end of each line (≤ 4 series)
            const ctx = u.ctx;
            const d = u.data;
            ctx.font = `${11 * devicePixelRatio}px ${c.font}`;
            ctx.fillStyle = c.inkSecondary;
            ctx.textBaseline = "middle";
            const names = ["today", "90 d", "all-time"];
            for (let s = 1; s <= 3; s++) {
              const ys = d[s] as (number | null)[] | undefined;
              if (!ys) continue;
              let j = ys.length - 1;
              while (j >= 0 && (ys[j] === null || ys[j] === undefined)) j--;
              const yv = ys[j];
              const xv = d[0][j];
              if (j < 0 || yv === null || yv === undefined || xv === undefined) continue;
              ctx.fillText(names[s - 1] ?? "", u.valToPos(xv, "x", true) + 4, u.valToPos(yv, "y", true) + (s - 2) * 6 * devicePixelRatio);
            }
          },
        ],
      },
    };
  };
  const hv = $derived.by(() => {
    const c = curve;
    const h = hover;
    if (!c || !h) return null;
    const d = c.durationsS[h.i];
    if (d === undefined) return null;
    const t = c.today[h.i] ?? 0;
    const a = c.allTime[h.i] ?? 0;
    return { d, today: t, d90: c.best90d[h.i] ?? 0, all: a, pct: a > 0 ? (t / a) * 100 : 0, left: h.left };
  });
</script>

<ChartFrame {title}>
  {#snippet legend()}
    <span><i class="sw" style:background="var(--series-power)"></i>today</span>
    <span><i class="sw" style:background="var(--ink-secondary)"></i>90 d</span>
    <span><i class="sw dash"></i>all-time</span>
  {/snippet}
  {#snippet chart()}
    <div class="plot" {@attach uplot(options, () => data)}></div>
    {#if hv}
      <div class="tip" style:left="{hv.left + 44}px">
        <b>{dur(hv.d)}</b> today {hv.today > 0 ? `${Math.round(hv.today)} W (${hv.pct.toFixed(0)}% of best)` : "—"}<br />
        <span class="muted">90 d {Math.round(hv.d90)} W · all-time {Math.round(hv.all)} W</span>
      </div>
    {/if}
  {/snippet}
  {#snippet table()}
    {#if curve}
      <table class="data">
        <thead><tr><th>duration</th><th>today</th><th>90 d</th><th>all-time</th></tr></thead>
        <tbody>
          {#each curve.durationsS as d, i (d)}
            <tr><td>{dur(d)}</td><td>{(curve.today[i] ?? 0) > 0 ? Math.round(curve.today[i] ?? 0) : "—"}</td><td>{Math.round(curve.best90d[i] ?? 0)}</td><td>{Math.round(curve.allTime[i] ?? 0)}</td></tr>
          {/each}
        </tbody>
      </table>
    {/if}
  {/snippet}
</ChartFrame>

<style>
  .plot {
    position: absolute;
    inset: 0;
    overflow: hidden;
  }
  .sw {
    display: inline-block;
    width: 12px;
    height: 3px;
    margin-right: 4px;
    vertical-align: middle;
    border-radius: 2px;
  }
  .sw.dash {
    background: repeating-linear-gradient(90deg, var(--ink-muted) 0 4px, transparent 4px 7px);
  }
  .tip {
    position: absolute;
    top: 0;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0.2rem 0.45rem;
    font-size: 0.72rem;
    pointer-events: none;
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
    z-index: 2;
  }
</style>
