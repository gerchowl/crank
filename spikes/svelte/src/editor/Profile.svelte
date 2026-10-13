<script lang="ts">
  import { shiftTarget, type FlatBlock, type PowerTarget } from "@neo/shared";
  import { canvas2d } from "../charts/attach.svelte";
  import { drawProfile, drawFtpGuides, tx, type ProfileGeom } from "../charts/profile";
  import { backend } from "../lib/backend";
  import { theme } from "../lib/theme.svelte";
  import { clock, dur } from "../lib/format";
  import { editor } from "./state.svelte";
  import { isPrefix, nodeAt } from "./tree";

  // P13 editable profile + P24 diff overlay. Drag a block's right edge → duration,
  // drag its top → power. Click selects the step (shift-click extends among siblings).
  const PAD = { l: 40, r: 10, t: 12, b: 20 };
  const EDGE = 6;

  let hover = $state<{ i: number; mode: "dur" | "pow" | "body" } | null>(null);
  let drag: { i: number; mode: "dur" | "pow"; x0: number; y0: number; s0: number; t0: PowerTarget; spx: number; wpx: number } | null = null;

  const blocks = $derived(editor.compiled.blocks);
  const preview = $derived(editor.previewCompiled);
  const ftp = $derived(editor.workout.ftp);

  function geom(w: number, h: number): ProfileGeom {
    const all = preview ? [...blocks, ...preview.blocks] : blocks;
    const end = Math.max(60, ...all.map((b) => b.startS + Math.max(b.durS, b.durRangeS?.[1] ?? 0)));
    const maxW = Math.max(ftp * 1.4, ...all.map((b) => Math.max(b.powerFrom, b.powerTo) * 1.08));
    return { x: PAD.l, y: PAD.t, w: w - PAD.l - PAD.r, h: h - PAD.t - PAD.b, t0: 0, t1: end, maxW };
  }

  const selectedBlock = (b: FlatBlock) => editor.selection.some((p) => isPrefix(p, b.path));

  function inProposal(b: FlatBlock): boolean {
    const p = editor.proposal;
    if (!p) return false;
    const d = p.parent.length;
    const i = b.path[d] ?? -1;
    return isPrefix(p.parent, b.path) && i >= p.lo && i < p.lo + p.nodes.length;
  }

  function draw(ctx: CanvasRenderingContext2D, { w, h }: { w: number; h: number }): void {
    const c = theme.colors;
    const g = geom(w, h);
    drawFtpGuides(ctx, g, ftp, c);
    if (preview) {
      // P24: new profile solid with changed blocks outlined, old profile ghosted on top
      drawProfile(ctx, preview.blocks, g, { colors: c, zones: backend.zones.power, changed: inProposal });
      drawProfile(ctx, blocks, g, { colors: c, zones: backend.zones.power, ghost: true });
    } else {
      drawProfile(ctx, blocks, g, {
        colors: c,
        zones: backend.zones.power,
        selected: (b) => selectedBlock(b),
        showAi: true,
      });
      if (hover && hover.mode !== "body") {
        const b = blocks[hover.i];
        if (b) {
          ctx.strokeStyle = c.accent;
          ctx.lineWidth = 3;
          ctx.beginPath();
          if (hover.mode === "dur") {
            const x = tx(g, b.startS + b.durS);
            ctx.moveTo(x, g.y);
            ctx.lineTo(x, g.y + g.h);
          } else {
            const x0 = tx(g, b.startS);
            const x1 = tx(g, b.startS + b.durS);
            const y = g.y + g.h - (Math.max(b.powerFrom, b.powerTo) / g.maxW) * g.h;
            ctx.moveTo(x0, y);
            ctx.lineTo(x1, y);
          }
          ctx.stroke();
        }
      }
    }
    // time axis
    ctx.fillStyle = c.muted;
    ctx.font = `10px ${c.font}`;
    ctx.textBaseline = "top";
    const step = [60, 120, 300, 600, 900].find((s) => (g.t1 / s) * 60 < g.w) ?? 1800;
    for (let s = 0; s <= g.t1; s += step) {
      ctx.textAlign = s === 0 ? "left" : "center";
      ctx.fillText(dur(s) || "0", tx(g, s), g.y + g.h + 5);
    }
    ctx.textAlign = "left";
  }

  function hit(e: PointerEvent): { i: number; mode: "dur" | "pow" | "body"; g: ProfileGeom } | null {
    const el = e.currentTarget as HTMLCanvasElement;
    const r = el.getBoundingClientRect();
    const g = geom(r.width, r.height);
    const x = e.clientX - r.left;
    const y = e.clientY - r.top;
    for (let i = 0; i < blocks.length; i++) {
      const b = blocks[i];
      if (!b) continue;
      const x0 = tx(g, b.startS);
      const x1 = tx(g, b.startS + b.durS);
      if (x < x0 - EDGE || x > x1 + EDGE) continue;
      const top = g.y + g.h - (Math.max(b.powerFrom, b.powerTo) / g.maxW) * g.h;
      if (Math.abs(x - x1) <= EDGE && y > top) return { i, mode: "dur", g };
      if (x >= x0 && x <= x1 && Math.abs(y - top) <= EDGE) return { i, mode: "pow", g };
      if (x >= x0 && x <= x1 && y >= top) return { i, mode: "body", g };
    }
    return null;
  }

  function down(e: PointerEvent): void {
    if (editor.proposal) return;
    const h = hit(e);
    if (!h) {
      editor.select(null);
      return;
    }
    const b = blocks[h.i];
    if (!b) return;
    editor.select(b.path, e.shiftKey);
    const node = nodeAt(editor.workout.nodes, b.path);
    if (h.mode === "body" || node?.type !== "step") return;
    if (h.mode === "dur" && node.end.kind !== "time") return;
    editor.checkpoint();
    drag = {
      i: h.i,
      mode: h.mode,
      x0: e.clientX,
      y0: e.clientY,
      s0: node.end.kind === "time" ? node.end.s : 0,
      t0: $state.snapshot(node.target),
      spx: (h.g.t1 - h.g.t0) / h.g.w,
      wpx: h.g.maxW / h.g.h,
    };
    (e.currentTarget as HTMLCanvasElement).setPointerCapture(e.pointerId);
  }

  function moveP(e: PointerEvent): void {
    if (!drag) {
      const h = editor.proposal ? null : hit(e);
      hover = h ? { i: h.i, mode: h.mode } : null;
      return;
    }
    const b = blocks[drag.i];
    const node = b ? nodeAt(editor.workout.nodes, b.path) : undefined;
    if (node?.type !== "step") return;
    if (drag.mode === "dur" && node.end.kind === "time") {
      const s = Math.max(5, Math.round((drag.s0 + (e.clientX - drag.x0) * drag.spx) / 5) * 5);
      if (s !== node.end.s) node.end.s = s;
    } else if (drag.mode === "pow") {
      const dPct = Math.round((((drag.y0 - e.clientY) * drag.wpx) / ftp) * 100);
      node.target = shiftTarget(drag.t0, dPct, ftp);
    }
  }

  function up(e: PointerEvent): void {
    if (drag) (e.currentTarget as HTMLCanvasElement).releasePointerCapture(e.pointerId);
    drag = null;
  }

  const hb = $derived(hover ? blocks[hover.i] : undefined);
</script>

<div class="wrap">
  <canvas
    class="cv"
    class:dur={hover?.mode === "dur"}
    class:pow={hover?.mode === "pow"}
    {@attach canvas2d(draw)}
    onpointerdown={down}
    onpointermove={moveP}
    onpointerup={up}
    onpointerleave={() => (hover = null)}
  ></canvas>
  <div class="info muted">
    {#if editor.proposal}
      <span>Diff: <b class="secondary">new solid</b>, old dashed, changed blocks outlined</span>
    {:else if hb}
      <span class="tabular">{clock(hb.startS)} · {dur(hb.durS)}{hb.durRangeS ? ` (${dur(hb.durRangeS[0])}–${dur(hb.durRangeS[1])})` : ""} · {Math.round(hb.powerFrom)}{hb.powerTo !== hb.powerFrom ? `→${Math.round(hb.powerTo)}` : ""} W · {hb.phase}</span>
      {#if hover?.mode === "dur"}<span>drag → duration</span>{:else if hover?.mode === "pow"}<span>drag ↕ power</span>{/if}
    {:else}
      <span>Click a block to select · drag its right edge for duration, its top for power</span>
    {/if}
  </div>
</div>

<style>
  .wrap {
    height: 100%;
    display: flex;
    flex-direction: column;
  }
  .cv {
    flex: 1;
    min-height: 0;
    width: 100%;
    display: block;
    touch-action: none;
  }
  .cv.dur {
    cursor: ew-resize;
  }
  .cv.pow {
    cursor: ns-resize;
  }
  .info {
    display: flex;
    gap: 1rem;
    font-size: 0.75rem;
    padding: 0.2rem 0.4rem 0;
    min-height: 1.3rem;
  }
</style>
