<script lang="ts">
  import type { HrRecoveryState } from "@neo/shared";
  import type uPlot from "uplot";
  import { uplot } from "./attach.svelte";
  import { axisStyle } from "./style";
  import { theme, withAlpha } from "../lib/theme.svelte";
  import { dur } from "../lib/format";

  // P12 HR recovery: observed dots, fitted curve (solid, over the observed span), dotted
  // prediction with 90 % band, neutral dashed target line. x = seconds since block start.
  let { state: rec }: { state: HrRecoveryState | undefined } = $props();

  const data = $derived.by((): uPlot.AlignedData => {
    const r = rec;
    if (!r || r.observed.length === 0) return [[], [], [], [], [], [], []];
    const t0 = r.startedT;
    // eslint-disable-next-line svelte/prefer-svelte-reactivity -- scratch map local to this derived, never stored
    const pts = new Map<number, { obs?: number | null; fit?: number; pred?: number; lo?: number; hi?: number }>();
    const key = (t: number) => Math.round((t - t0) * 4) / 4;
    for (const s of r.observed) pts.set(key(s.t), { obs: s.v });
    const f = r.fit;
    if (f) {
      for (const s of r.observed) {
        if (s.t < r.peakT) continue;
        const p = pts.get(key(s.t)) ?? {};
        p.fit = f.hrInf + f.amp * Math.exp(-(s.t - r.peakT) / f.tauS);
        pts.set(key(s.t), p);
      }
    }
    for (const p of r.predicted ?? []) {
      const e = pts.get(key(p.t)) ?? {};
      e.pred = p.mean;
      e.lo = p.lo;
      e.hi = p.hi;
      pts.set(key(p.t), e);
    }
    const xs = [...pts.keys()].sort((a, b) => a - b);
    const g = (k: "obs" | "fit" | "pred" | "lo" | "hi") => xs.map((x) => pts.get(x)?.[k] ?? null);
    return [xs, g("obs"), g("fit"), g("pred"), g("lo"), g("hi"), xs.map(() => r.targetBpm)];
  });

  const xMax = $derived(Math.max(rec?.maxS ?? 240, ...((data[0] as number[]).length ? [(data[0] as number[]).at(-1) ?? 0] : [])));

  const options = (): Omit<uPlot.Options, "width" | "height"> => {
    const c = theme.colors;
    return {
      legend: { show: false },
      cursor: { y: false, points: { show: false } },
      scales: {
        x: { time: false, range: () => [0, xMax] },
        y: { auto: true, range: (_u, lo, hi) => [Math.floor(Math.min(lo, rec?.targetBpm ?? lo) - 8), Math.ceil(hi + 6)] },
      },
      axes: [
        axisStyle(c, { size: 24, values: (_u, s) => s.map((x: number | null) => (x === null ? "" : dur(x))) // uPlot nulls filtered log ticks at runtime, incrs: [15, 30, 60, 120] }),
        axisStyle(c, { size: 36 }),
      ],
      series: [
        {},
        { stroke: "transparent", width: 0, points: { show: true, size: 5, fill: c.hr, stroke: c.hr, width: 0 } },
        { stroke: c.hr, width: 2, points: { show: false } },
        { stroke: c.hr, width: 2, dash: [2, 4], points: { show: false } },
        { stroke: "transparent", points: { show: false } },
        { stroke: "transparent", points: { show: false } },
        { stroke: c.target, width: 1.5, dash: [6, 5], points: { show: false } },
      ],
      bands: [{ series: [5, 4], fill: withAlpha(c.hr, 0.18) }],
      padding: [10, 12, 0, 0],
    };
  };
</script>

<div class="plot" {@attach uplot(options, () => data)}></div>

<style>
  .plot {
    position: absolute;
    inset: 0;
    overflow: hidden;
  }
</style>
