<script lang="ts">
  import type { Heatmap2D } from "@neo/shared";
  import { canvas2d } from "./attach.svelte";
  import { theme, rampAt } from "../lib/theme.svelte";
  import { label, unit, zonesFor, clock } from "../lib/format";
  import ChartFrame from "./ChartFrame.svelte";
  import type { Snippet } from "svelte";

  // P07 2D heatmap: sequential single-hue fill = seconds (the preset's zone hue ramp, which
  // is light→dark on light and dark→light on dark), zone gridlines, per-cell hover, table.
  let { map, title, controls }: { map: Heatmap2D | undefined; title: string; controls?: Snippet | undefined } = $props();

  const PAD = { l: 34, r: 6, t: 6, b: 18 };
  let hover = $state<{ col: number; row: number; px: number; py: number } | null>(null);

  // Crop to the occupied cells.
  const view = $derived.by(() => {
    const m = map;
    if (!m || m.cols === 0 || m.rows === 0) return null;
    let c0 = m.cols;
    let c1 = -1;
    let r0 = m.rows;
    let r1 = -1;
    let max = 0;
    for (let r = 0; r < m.rows; r++)
      for (let c = 0; c < m.cols; c++) {
        const s = m.seconds[r * m.cols + c] ?? 0;
        if (s > 0) {
          c0 = Math.min(c0, c);
          c1 = Math.max(c1, c);
          r0 = Math.min(r0, r);
          r1 = Math.max(r1, r);
          max = Math.max(max, s);
        }
      }
    if (c1 < 0) return null;
    return { c0, c1, r0, r1, max };
  });

  function draw(ctx: CanvasRenderingContext2D, { w, h }: { w: number; h: number }): void {
    const c = theme.colors;
    const m = map;
    const v = view;
    if (!m || !v) {
      ctx.fillStyle = c.muted;
      ctx.font = `12px ${c.font}`;
      ctx.fillText("no data yet", PAD.l, h / 2);
      return;
    }
    const nc = v.c1 - v.c0 + 1;
    const nr = v.r1 - v.r0 + 1;
    const cw = (w - PAD.l - PAD.r) / nc;
    const ch = (h - PAD.t - PAD.b) / nr;
    // zone ramp runs "near surface" → "max contrast" in both modes (dark steps are selected, not inverted)
    const stops = c.zones;
    for (let r = v.r0; r <= v.r1; r++)
      for (let col = v.c0; col <= v.c1; col++) {
        const s = m.seconds[r * m.cols + col] ?? 0;
        if (s <= 0) continue;
        const t = Math.sqrt(s / v.max);
        ctx.fillStyle = rampAt(stops, t);
        const x = PAD.l + (col - v.c0) * cw;
        const y = PAD.t + (v.r1 - r) * ch;
        ctx.fillRect(x + 0.5, y + 0.5, Math.max(1, cw - 1), Math.max(1, ch - 1));
      }
    // zone gridlines
    ctx.strokeStyle = c.axis;
    ctx.setLineDash([3, 3]);
    ctx.lineWidth = 1;
    const xLo = m.xLo + v.c0 * m.xBin;
    const xHi = m.xLo + (v.c1 + 1) * m.xBin;
    const yLo = m.yLo + v.r0 * m.yBin;
    const yHi = m.yLo + (v.r1 + 1) * m.yBin;
    const X = (val: number) => PAD.l + ((val - xLo) / (xHi - xLo)) * (w - PAD.l - PAD.r);
    const Y = (val: number) => PAD.t + (1 - (val - yLo) / (yHi - yLo)) * (h - PAD.t - PAD.b);
    ctx.fillStyle = c.muted;
    ctx.font = `10px ${c.font}`;
    for (const z of zonesFor(m.x) ?? []) {
      if (z.min <= xLo || z.min >= xHi) continue;
      ctx.beginPath();
      ctx.moveTo(Math.round(X(z.min)) + 0.5, PAD.t);
      ctx.lineTo(Math.round(X(z.min)) + 0.5, h - PAD.b);
      ctx.stroke();
      ctx.fillText(z.name.split(" ")[0] ?? "", X(z.min) + 2, PAD.t + 9);
    }
    for (const z of zonesFor(m.y) ?? []) {
      if (z.min <= yLo || z.min >= yHi) continue;
      ctx.beginPath();
      ctx.moveTo(PAD.l, Math.round(Y(z.min)) + 0.5);
      ctx.lineTo(w - PAD.r, Math.round(Y(z.min)) + 0.5);
      ctx.stroke();
      ctx.fillText(z.name.split(" ")[0] ?? "", w - PAD.r - 16, Y(z.min) - 3);
    }
    ctx.setLineDash([]);
    // axis labels
    ctx.textAlign = "center";
    ctx.textBaseline = "top";
    ctx.fillText(`${Math.round(xLo)}`, PAD.l, h - PAD.b + 4);
    ctx.fillText(`${Math.round(xHi)} ${unit(m.x)}`, w - PAD.r - 24, h - PAD.b + 4);
    ctx.textAlign = "right";
    ctx.textBaseline = "middle";
    ctx.fillText(`${Math.round(yHi)}`, PAD.l - 4, PAD.t + 6);
    ctx.fillText(`${Math.round(yLo)}`, PAD.l - 4, h - PAD.b - 6);
    if (hover) {
      ctx.strokeStyle = c.ink;
      ctx.lineWidth = 2;
      ctx.strokeRect(PAD.l + (hover.col - v.c0) * cw, PAD.t + (v.r1 - hover.row) * ch, cw, ch);
    }
  }

  function onMove(e: PointerEvent): void {
    const m = map;
    const v = view;
    if (!m || !v) return;
    const r = (e.currentTarget as HTMLCanvasElement).getBoundingClientRect();
    const nc = v.c1 - v.c0 + 1;
    const nr = v.r1 - v.r0 + 1;
    const col = v.c0 + Math.floor(((e.clientX - r.left - PAD.l) / (r.width - PAD.l - PAD.r)) * nc);
    const row = v.r1 - Math.floor(((e.clientY - r.top - PAD.t) / (r.height - PAD.t - PAD.b)) * nr);
    if (col < v.c0 || col > v.c1 || row < v.r0 || row > v.r1) hover = null;
    else hover = { col, row, px: e.clientX - r.left, py: e.clientY - r.top };
  }
</script>

<ChartFrame {title} {controls}>
  {#snippet legend()}
    {#if map}<span class="muted">{label(map.x)} × {label(map.y)} · fill = time</span>{/if}
  {/snippet}
  {#snippet chart()}
    <canvas class="cv" {@attach canvas2d(draw)} onpointermove={onMove} onpointerleave={() => (hover = null)}></canvas>
    {#if hover && map}
      {@const s = map.seconds[hover.row * map.cols + hover.col] ?? 0}
      <div class="tip" style:left="{hover.px + 10}px" style:top="{hover.py}px">
        {label(map.x)} {Math.round(map.xLo + hover.col * map.xBin)}–{Math.round(map.xLo + (hover.col + 1) * map.xBin)} {unit(map.x)}<br />
        {label(map.y)} {Math.round(map.yLo + hover.row * map.yBin)}–{Math.round(map.yLo + (hover.row + 1) * map.yBin)} {unit(map.y)}<br />
        <b>{clock(s)}</b>
      </div>
    {/if}
  {/snippet}
  {#snippet table()}
    {#if map && view}
      <table class="data">
        <thead><tr><th>{label(map.x)}</th><th>{label(map.y)}</th><th>time</th></tr></thead>
        <tbody>
          {#each Array.from({ length: (view.r1 - view.r0 + 1) * (view.c1 - view.c0 + 1) }, (_, k) => k) as k (k)}
            {@const row = view.r0 + Math.floor(k / (view.c1 - view.c0 + 1))}
            {@const col = view.c0 + (k % (view.c1 - view.c0 + 1))}
            {@const s = map.seconds[row * map.cols + col] ?? 0}
            {#if s > 0}
              <tr><td>{Math.round(map.xLo + col * map.xBin)}</td><td>{Math.round(map.yLo + row * map.yBin)}</td><td>{clock(s)}</td></tr>
            {/if}
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
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0.2rem 0.45rem;
    font-size: 0.72rem;
    pointer-events: none;
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
</style>
