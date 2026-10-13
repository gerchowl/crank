<script lang="ts">
  import type { Widget } from "@neo/shared";
  import { canvas2d } from "../../charts/attach.svelte";
  import { drawProfile, drawFtpGuides, tx, type ProfileGeom } from "../../charts/profile";
  import { liveMetric } from "../../lib/live.svelte";
  import { ride } from "../../lib/ride.svelte";
  import { backend } from "../../lib/backend";
  import { theme } from "../../lib/theme.svelte";
  import { clock, PHASE_LABEL } from "../../lib/format";

  // P13 strip: full profile (or a look-ahead window when spanS is set), "you are here"
  // marker, AI-changed blocks outlined, done blocks dimmed, click a future block → jump.
  let { widget }: { widget: Widget } = $props();

  // Position on the *plan*, not wall time: skips and HR-guided blocks make elapsed drift
  // from the planned timeline, so "you are here" = block start + progress within the block.
  const blockRemaining = liveMetric(
    () => "block_remaining",
    () => ({ rateHz: widget.rateHz }),
  );
  const t = $derived.by(() => {
    const b = ride.block;
    if (!b) return ride.state === "finished" ? ride.totalS : 0;
    const rem = blockRemaining.v;
    return b.startS + Math.max(0, Math.min(b.durS, rem === null ? 0 : b.durS - rem));
  });
  const ftp = $derived(ride.current?.workout.ftp ?? backend.athlete.ftp);
  const window_ = $derived.by((): [number, number] => {
    const total = Math.max(60, ride.totalS);
    if (!widget.spanS) return [0, total];
    const t0 = Math.max(0, t - widget.spanS * 0.15);
    return [t0, t0 + widget.spanS];
  });
  const PAD = { l: 34, r: 8, t: 14, b: 16 };
  let hoverI = $state<number | null>(null);

  function geom(w: number, h: number): ProfileGeom {
    const maxW = Math.max(ftp * 1.3, ...ride.blocks.map((b) => Math.max(b.powerFrom, b.powerTo) * 1.05));
    return { x: PAD.l, y: PAD.t, w: w - PAD.l - PAD.r, h: h - PAD.t - PAD.b, t0: window_[0], t1: window_[1], maxW };
  }

  function draw(ctx: CanvasRenderingContext2D, { w, h }: { w: number; h: number }): void {
    const c = theme.colors;
    const g = geom(w, h);
    drawFtpGuides(ctx, g, ftp, c);
    drawProfile(ctx, ride.blocks, g, {
      colors: c,
      zones: backend.zones.power,
      showAi: true,
      doneBefore: t,
      selected: (_b, i) => i === hoverI,
    });
    // you-are-here marker
    const x = Math.round(tx(g, t)) + 0.5;
    ctx.strokeStyle = c.ink;
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(x, PAD.t - 4);
    ctx.lineTo(x, h - PAD.b);
    ctx.stroke();
    ctx.fillStyle = c.ink;
    ctx.beginPath();
    ctx.moveTo(x - 5, PAD.t - 10);
    ctx.lineTo(x + 5, PAD.t - 10);
    ctx.lineTo(x, PAD.t - 3);
    ctx.closePath();
    ctx.fill();
    // time labels
    ctx.fillStyle = c.muted;
    ctx.font = `10px ${c.font}`;
    ctx.textBaseline = "top";
    ctx.textAlign = "left";
    ctx.fillText(clock(g.t0), g.x, h - PAD.b + 3);
    ctx.textAlign = "right";
    ctx.fillText(clock(g.t1), g.x + g.w, h - PAD.b + 3);
    ctx.textAlign = "left";
  }

  function blockAt(e: PointerEvent | MouseEvent): number | null {
    const el = e.currentTarget as HTMLCanvasElement;
    const r = el.getBoundingClientRect();
    const g = geom(r.width, r.height);
    const time = g.t0 + ((e.clientX - r.left - g.x) / g.w) * (g.t1 - g.t0);
    const i = ride.blocks.findIndex((b) => time >= b.startS && time < b.startS + b.durS);
    return i >= 0 ? i : null;
  }

  function jump(e: MouseEvent): void {
    const i = blockAt(e);
    if (i === null || i <= ride.blockIndex) return;
    // The contract has no seek; a jump is N skips.
    for (let k = ride.blockIndex; k < i; k++) backend.ride.skipBlock();
  }
  const hb = $derived(hoverI !== null ? ride.blocks[hoverI] : undefined);
</script>

<div class="strip">
  <div class="head">
    <span class="title">{ride.current?.workout.name ?? "No workout"}</span>
    <span class="secondary">{PHASE_LABEL[ride.phase]} · block {ride.blockIndex + 1}/{ride.blocks.length}</span>
    {#if hb}<span class="muted">· hover: {clock(hb.durS)} @ {Math.round(hb.powerFrom)}{hb.powerTo !== hb.powerFrom ? `→${Math.round(hb.powerTo)}` : ""} W{hb.aiChanged ? " (AI changed)" : ""}{hoverI !== null && hoverI > ride.blockIndex ? " — click to jump" : ""}</span>{/if}
  </div>
  <canvas
    class="cv"
    {@attach canvas2d(draw)}
    onpointermove={(e) => (hoverI = blockAt(e))}
    onpointerleave={() => (hoverI = null)}
    onclick={jump}
  ></canvas>
</div>

<style>
  .strip {
    height: 100%;
    display: flex;
    flex-direction: column;
    padding: 0.35rem 0.6rem 0.2rem;
  }
  .head {
    display: flex;
    gap: 0.6rem;
    font-size: 0.75rem;
    white-space: nowrap;
    overflow: hidden;
  }
  .title {
    letter-spacing: 0.02em;
    font-weight: 600;
    color: var(--ink-secondary);
  }
  .cv {
    flex: 1;
    min-height: 0;
    width: 100%;
    display: block;
    cursor: pointer;
  }
</style>
