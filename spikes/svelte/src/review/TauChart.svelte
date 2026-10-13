<script lang="ts">
  import type { HrRecoveryState } from "@neo/shared";
  import { canvas2d } from "../charts/attach.svelte";
  import ChartFrame from "../charts/ChartFrame.svelte";
  import { theme } from "../lib/theme.svelte";
  import { clock } from "../lib/format";

  // P12 per block: τ of every finished HR-recovery block (dots, HR series hue) vs the usual τ
  // (neutral dashed reference). Hover + table.
  let { recs }: { recs: HrRecoveryState[] } = $props();
  const PAD = { l: 36, r: 10, t: 10, b: 20 };
  const fits = $derived(recs.flatMap((r, i) => (r.fit ? [{ n: i + 1, tau: r.fit.tauS, usual: r.fit.usualTauS, t: r.startedT, unreachable: r.unreachable }] : [])));
  let hover = $state<number | null>(null);

  function geom(w: number, h: number) {
    const max = Math.max(80, ...fits.map((f) => Math.max(f.tau, f.usual) * 1.2));
    const n = Math.max(3, fits.length);
    return {
      X: (i: number) => PAD.l + ((i - 0.5) / n) * (w - PAD.l - PAD.r),
      Y: (v: number) => PAD.t + (1 - v / max) * (h - PAD.t - PAD.b),
      max,
    };
  }

  function draw(ctx: CanvasRenderingContext2D, { w, h }: { w: number; h: number }): void {
    const c = theme.colors;
    const g = geom(w, h);
    ctx.font = `10px ${c.font}`;
    ctx.fillStyle = c.muted;
    ctx.strokeStyle = c.grid;
    for (let v = 0; v <= g.max; v += 20) {
      const y = Math.round(g.Y(v)) + 0.5;
      ctx.beginPath();
      ctx.moveTo(PAD.l, y);
      ctx.lineTo(w - PAD.r, y);
      ctx.stroke();
      ctx.textAlign = "right";
      ctx.fillText(`${v}s`, PAD.l - 4, y + 3);
    }
    if (!fits.length) {
      ctx.textAlign = "left";
      ctx.fillText("no finished HR-recovery blocks yet", PAD.l + 6, h / 2);
      return;
    }
    const usual = fits[0]?.usual ?? 0;
    ctx.strokeStyle = c.target;
    ctx.setLineDash([6, 4]);
    ctx.beginPath();
    ctx.moveTo(PAD.l, g.Y(usual));
    ctx.lineTo(w - PAD.r, g.Y(usual));
    ctx.stroke();
    ctx.setLineDash([]);
    ctx.textAlign = "right";
    ctx.fillText(`usual ${Math.round(usual)} s`, w - PAD.r, g.Y(usual) - 4);
    for (const f of fits) {
      ctx.fillStyle = c.hr;
      ctx.beginPath();
      ctx.arc(g.X(f.n), g.Y(f.tau), hover === f.n ? 7 : 5, 0, Math.PI * 2);
      ctx.fill();
      ctx.fillStyle = c.inkSecondary;
      ctx.textAlign = "center";
      ctx.fillText(`#${f.n}`, g.X(f.n), h - 6);
    }
  }

  function onMove(e: PointerEvent): void {
    const r = (e.currentTarget as HTMLCanvasElement).getBoundingClientRect();
    const g = geom(r.width, r.height);
    const mx = e.clientX - r.left;
    const near = fits.find((f) => Math.abs(g.X(f.n) - mx) < 14);
    hover = near ? near.n : null;
  }
  const hf = $derived(fits.find((f) => f.n === hover));
</script>

<ChartFrame title="HR recovery τ per block">
  {#snippet legend()}
    <span><i class="dot"></i>τ</span><span><i class="dash"></i>usual τ</span>
  {/snippet}
  {#snippet chart()}
    <canvas class="cv" {@attach canvas2d(draw)} onpointermove={onMove} onpointerleave={() => (hover = null)}></canvas>
    {#if hf}<div class="tip">#{hf.n} at {clock(hf.t)} · τ {Math.round(hf.tau)} s (usual {Math.round(hf.usual)} s){hf.unreachable ? " · target unreachable" : ""}</div>{/if}
  {/snippet}
  {#snippet table()}
    <table class="data">
      <thead><tr><th>block</th><th>start</th><th>τ</th><th>usual</th></tr></thead>
      <tbody>
        {#each fits as f (f.n)}<tr><td>#{f.n}</td><td>{clock(f.t)}</td><td>{Math.round(f.tau)} s</td><td>{Math.round(f.usual)} s</td></tr>{/each}
      </tbody>
    </table>
  {/snippet}
</ChartFrame>

<style>
  .cv {
    display: block;
    width: 100%;
    height: 100%;
  }
  .dot {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--series-hr);
    margin-right: 3px;
  }
  .dash {
    display: inline-block;
    width: 14px;
    border-top: 2px dashed var(--target);
    margin-right: 3px;
    vertical-align: middle;
  }
  .tip {
    position: absolute;
    top: 0;
    left: 40px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0.15rem 0.45rem;
    font-size: 0.72rem;
  }
</style>
