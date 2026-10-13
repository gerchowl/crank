<script lang="ts">
  import { canvas2d } from "../charts/attach.svelte";
  import ChartFrame from "../charts/ChartFrame.svelte";
  import { theme } from "../lib/theme.svelte";
  import { clock } from "../lib/format";
  import type { RideRecord } from "./record.svelte";

  // P16 power × HR, point colour = kJ spent (fatigue ramp, a different hue than zones).
  // Falls back to density cells above 5k points. Nearest-point hover.
  let { rec }: { rec: RideRecord } = $props();
  const PAD = { l: 36, r: 8, t: 8, b: 20 };

  const pts = $derived.by(() => {
    rec.track();
    const p = rec.get("power");
    const h = rec.get("hr");
    const k = rec.get("kj");
    const out: { x: number; y: number; kj: number; t: number }[] = [];
    for (let i = 0; i < rec.t.length; i++) {
      const x = p[i];
      const y = h[i];
      if (x === null || x === undefined || y === null || y === undefined || x <= 0) continue;
      out.push({ x, y, kj: k[i] ?? 0, t: rec.t[i] ?? i });
    }
    return out;
  });
  const ext = $derived.by(() => {
    if (!pts.length) return null;
    let x1 = 0;
    let y0 = Infinity;
    let y1 = 0;
    let k1 = 0;
    for (const p of pts) {
      x1 = Math.max(x1, p.x);
      y0 = Math.min(y0, p.y);
      y1 = Math.max(y1, p.y);
      k1 = Math.max(k1, p.kj);
    }
    return { x0: 0, x1: Math.ceil(x1 / 50) * 50, y0: Math.floor(y0 / 10) * 10, y1: Math.ceil(y1 / 10) * 10, k1: Math.max(1, k1) };
  });
  let hover = $state<{ i: number; px: number; py: number } | null>(null);

  const step = (kj: number, k1: number) => Math.min(4, Math.floor((kj / k1) * 5));

  function draw(ctx: CanvasRenderingContext2D, { w, h }: { w: number; h: number }): void {
    const c = theme.colors;
    const e = ext;
    if (!e) {
      ctx.fillStyle = c.muted;
      ctx.font = `12px ${c.font}`;
      ctx.fillText("no data yet", PAD.l, h / 2);
      return;
    }
    const X = (v: number) => PAD.l + ((v - e.x0) / (e.x1 - e.x0)) * (w - PAD.l - PAD.r);
    const Y = (v: number) => PAD.t + (1 - (v - e.y0) / (e.y1 - e.y0)) * (h - PAD.t - PAD.b);
    ctx.strokeStyle = c.grid;
    ctx.fillStyle = c.muted;
    ctx.font = `10px ${c.font}`;
    for (let v = e.y0; v <= e.y1; v += 20) {
      ctx.beginPath();
      ctx.moveTo(PAD.l, Math.round(Y(v)) + 0.5);
      ctx.lineTo(w - PAD.r, Math.round(Y(v)) + 0.5);
      ctx.stroke();
      ctx.textAlign = "right";
      ctx.fillText(String(v), PAD.l - 4, Y(v) + 3);
    }
    ctx.textAlign = "center";
    for (let v = 0; v <= e.x1; v += 100) ctx.fillText(`${v}`, X(v), h - 6);
    if (pts.length > 5000) {
      // density fallback: 4 px cells, colour = mean kJ step, alpha = count
      const cell = 4;
      // eslint-disable-next-line svelte/prefer-svelte-reactivity -- scratch accumulator inside a draw call
      const acc = new Map<number, { n: number; kj: number }>();
      for (const p of pts) {
        const k = Math.floor(X(p.x) / cell) * 10000 + Math.floor(Y(p.y) / cell);
        const a = acc.get(k) ?? { n: 0, kj: 0 };
        a.n++;
        a.kj += p.kj;
        acc.set(k, a);
      }
      for (const [k, a] of acc) {
        ctx.globalAlpha = Math.min(1, 0.25 + a.n / 6);
        ctx.fillStyle = c.fatigue[step(a.kj / a.n, e.k1)] ?? c.muted;
        ctx.fillRect(Math.floor(k / 10000) * cell, (k % 10000) * cell, cell, cell);
      }
      ctx.globalAlpha = 1;
    } else {
      for (const p of pts) {
        ctx.fillStyle = c.fatigue[step(p.kj, e.k1)] ?? c.muted;
        ctx.beginPath();
        ctx.arc(X(p.x), Y(p.y), 2.2, 0, Math.PI * 2);
        ctx.fill();
      }
    }
    const hp = hover ? pts[hover.i] : undefined;
    if (hp) {
      ctx.strokeStyle = c.ink;
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.arc(X(hp.x), Y(hp.y), 5, 0, Math.PI * 2);
      ctx.stroke();
    }
  }

  function onMove(e: PointerEvent): void {
    const ex = ext;
    if (!ex) return;
    const r = (e.currentTarget as HTMLCanvasElement).getBoundingClientRect();
    const X = (v: number) => PAD.l + ((v - ex.x0) / (ex.x1 - ex.x0)) * (r.width - PAD.l - PAD.r);
    const Y = (v: number) => PAD.t + (1 - (v - ex.y0) / (ex.y1 - ex.y0)) * (r.height - PAD.t - PAD.b);
    const mx = e.clientX - r.left;
    const my = e.clientY - r.top;
    let best = -1;
    let bd = 64;
    pts.forEach((p, i) => {
      const d = (X(p.x) - mx) ** 2 + (Y(p.y) - my) ** 2;
      if (d < bd) {
        bd = d;
        best = i;
      }
    });
    hover = best >= 0 ? { i: best, px: mx, py: my } : null;
  }
  const hp = $derived(hover ? pts[hover.i] : undefined);
</script>

<ChartFrame title="Power × HR by kJ spent">
  {#snippet legend()}
    {#if ext}
      {#each theme.colors.fatigue as f, i (i)}
        <span><i class="sw" style:background={f}></i>{Math.round((ext.k1 * i) / 5)}–{Math.round((ext.k1 * (i + 1)) / 5)}</span>
      {/each}
      <span class="muted">kJ</span>
    {/if}
  {/snippet}
  {#snippet chart()}
    <canvas class="cv" {@attach canvas2d(draw)} onpointermove={onMove} onpointerleave={() => (hover = null)}></canvas>
    {#if hp && hover}
      <div class="tip" style:left="{hover.px + 10}px" style:top="{hover.py}px">{clock(hp.t)} · {Math.round(hp.x)} W · {Math.round(hp.y)} bpm · {Math.round(hp.kj)} kJ</div>
    {/if}
  {/snippet}
</ChartFrame>

<style>
  .cv {
    display: block;
    width: 100%;
    height: 100%;
  }
  .sw {
    display: inline-block;
    width: 9px;
    height: 9px;
    border-radius: 50%;
    margin-right: 3px;
  }
  .tip {
    position: absolute;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0.15rem 0.45rem;
    font-size: 0.72rem;
    pointer-events: none;
    white-space: nowrap;
  }
</style>
