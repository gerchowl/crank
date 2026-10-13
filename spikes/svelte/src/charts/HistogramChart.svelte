<script lang="ts">
  import type { Histogram } from "@neo/shared";
  import { canvas2d } from "./attach.svelte";
  import { theme, zoneColor, seriesColor } from "../lib/theme.svelte";
  import { zoneOf, unit, clock } from "../lib/format";
  import ChartFrame from "./ChartFrame.svelte";

  // P06 time-in-bin histogram: bars (2 px gaps, 4 px rounded tops) filled from the zone ramp,
  // "typical" as an outline step line, per-bar hover tooltip, table view.
  let { hist, title, colorByZone = true, showTypical = $bindable(true) }: { hist: Histogram | undefined; title: string; colorByZone?: boolean; showTypical?: boolean } = $props();

  let hoverIdx = $state<number | null>(null);
  let hoverX = $state(0);

  // Trim empty tails so the bars use the width.
  const view = $derived.by(() => {
    if (!hist) return null;
    const b = hist.bins;
    let a = b.findIndex((x) => x.seconds > 0);
    let z = b.findLastIndex((x) => x.seconds > 0);
    if (hist.typical && showTypical) {
      const ta = hist.typical.findIndex((x) => x > 1);
      const tz = hist.typical.findLastIndex((x) => x > 1);
      if (ta >= 0) a = a < 0 ? ta : Math.min(a, ta);
      if (tz >= 0) z = Math.max(z, tz);
    }
    if (a < 0) return null;
    return { bins: b.slice(a, z + 1), typical: hist.typical?.slice(a, z + 1), bw: hist.binWidth };
  });
  const PAD = { l: 34, r: 6, t: 6, b: 18 };

  function draw(ctx: CanvasRenderingContext2D, { w, h }: { w: number; h: number }): void {
    const c = theme.colors;
    const v = view;
    if (!v || !hist) {
      ctx.fillStyle = c.muted;
      ctx.font = `12px ${c.font}`;
      ctx.fillText("no data yet", PAD.l, h / 2);
      return;
    }
    const n = v.bins.length;
    const maxS = Math.max(1, ...v.bins.map((b) => b.seconds), ...(showTypical && v.typical ? v.typical : [0]));
    const pw = w - PAD.l - PAD.r;
    const ph = h - PAD.t - PAD.b;
    const bwPx = pw / n;
    const y = (s: number) => PAD.t + ph - (s / maxS) * ph;
    // grid
    ctx.strokeStyle = c.grid;
    ctx.lineWidth = 1;
    ctx.fillStyle = c.muted;
    ctx.font = `10px ${c.font}`;
    ctx.textAlign = "right";
    ctx.textBaseline = "middle";
    for (const f of [0.5, 1]) {
      const yy = Math.round(y(maxS * f)) + 0.5;
      ctx.beginPath();
      ctx.moveTo(PAD.l, yy);
      ctx.lineTo(w - PAD.r, yy);
      ctx.stroke();
      ctx.fillText(clock(maxS * f), PAD.l - 4, yy);
    }
    // bars
    v.bins.forEach((b, i) => {
      const x0 = PAD.l + i * bwPx + 1;
      const bw = Math.max(1, bwPx - 2);
      const top = y(b.seconds);
      const z = zoneOf(hist.metric, b.lo + v.bw / 2);
      ctx.fillStyle = colorByZone && z ? zoneColor(c, z.index, z.n) : seriesColor(c, hist.metric);
      if (hoverIdx !== null && hoverIdx !== i) ctx.globalAlpha = 0.55;
      const r = Math.min(4, bw / 2, PAD.t + ph - top);
      ctx.beginPath();
      ctx.roundRect(x0, top, bw, PAD.t + ph - top, [r, r, 0, 0]);
      ctx.fill();
      ctx.globalAlpha = 1;
    });
    // typical step line
    if (showTypical && v.typical) {
      ctx.strokeStyle = c.ink;
      ctx.lineWidth = 1.5;
      ctx.setLineDash([]);
      ctx.beginPath();
      v.typical.forEach((s, i) => {
        const x0 = PAD.l + i * bwPx;
        const yy = y(s);
        if (i === 0) ctx.moveTo(x0, yy);
        else ctx.lineTo(x0, yy);
        ctx.lineTo(x0 + bwPx, yy);
      });
      ctx.stroke();
    }
    // baseline + x labels
    ctx.strokeStyle = c.axis;
    ctx.beginPath();
    ctx.moveTo(PAD.l, PAD.t + ph + 0.5);
    ctx.lineTo(w - PAD.r, PAD.t + ph + 0.5);
    ctx.stroke();
    ctx.fillStyle = c.muted;
    ctx.textAlign = "center";
    ctx.textBaseline = "top";
    const every = Math.max(1, Math.ceil(n / Math.max(2, Math.floor(pw / 44))));
    v.bins.forEach((b, i) => {
      if (i % every === 0) ctx.fillText(String(Math.round(b.lo)), PAD.l + i * bwPx, PAD.t + ph + 4);
    });
  }

  function onMove(e: PointerEvent): void {
    const el = e.currentTarget as HTMLCanvasElement;
    const r = el.getBoundingClientRect();
    const v = view;
    if (!v) return;
    const i = Math.floor(((e.clientX - r.left - PAD.l) / (r.width - PAD.l - PAD.r)) * v.bins.length);
    hoverIdx = i >= 0 && i < v.bins.length ? i : null;
    hoverX = e.clientX - r.left;
  }
  const hb = $derived(hoverIdx !== null ? view?.bins[hoverIdx] : undefined);
</script>

<ChartFrame {title}>
  {#snippet legend()}
    {#if hist?.typical}
      <label class="typ"><input type="checkbox" bind:checked={showTypical} /> typical <i></i></label>
    {/if}
  {/snippet}
  {#snippet chart()}
    <canvas class="cv" {@attach canvas2d(draw)} onpointermove={onMove} onpointerleave={() => (hoverIdx = null)}></canvas>
    {#if hb && view && hist}
      <div class="tip" style:left="{hoverX}px">
        <b>{Math.round(hb.lo)}–{Math.round(hb.lo + view.bw)} {unit(hist.metric)}</b><br />
        {clock(hb.seconds)}
        {#if showTypical && view.typical && hoverIdx !== null}<span class="muted"> · typical {clock(view.typical[hoverIdx] ?? 0)}</span>{/if}
      </div>
    {/if}
  {/snippet}
  {#snippet table()}
    {#if view && hist}
      <table class="data">
        <thead><tr><th>bin ({unit(hist.metric)})</th><th>time</th>{#if view.typical}<th>typical</th>{/if}</tr></thead>
        <tbody>
          {#each view.bins as b, i (b.lo)}
            <tr><td>{Math.round(b.lo)}–{Math.round(b.lo + view.bw)}</td><td>{clock(b.seconds)}</td>{#if view.typical}<td>{clock(view.typical[i] ?? 0)}</td>{/if}</tr>
          {/each}
        </tbody>
      </table>
    {/if}
  {/snippet}
</ChartFrame>

<style>
  .cv {
    display: block;
    width: 100%;
    height: 100%;
  }
  .tip {
    position: absolute;
    top: 0;
    transform: translateX(-50%);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0.2rem 0.45rem;
    font-size: 0.72rem;
    pointer-events: none;
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
  .typ {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
  }
  .typ i {
    width: 14px;
    border-top: 2px solid var(--ink-primary);
  }
</style>
