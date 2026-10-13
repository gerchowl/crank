<script lang="ts">
  import type { CompiledWorkout } from "@neo/shared";
  import { canvas2d } from "./attach.svelte";
  import { drawProfile } from "./profile";
  import { backend } from "../lib/backend";
  import { theme } from "../lib/theme.svelte";

  // Small P13 profile for cards (proposal, review background).
  let { compiled, ftp }: { compiled: CompiledWorkout; ftp: number } = $props();

  function draw(ctx: CanvasRenderingContext2D, { w, h }: { w: number; h: number }): void {
    const end = Math.max(60, ...compiled.blocks.map((b) => b.startS + Math.max(b.durS, b.durRangeS?.[1] ?? 0)));
    const maxW = Math.max(ftp * 1.3, ...compiled.blocks.map((b) => Math.max(b.powerFrom, b.powerTo) * 1.05));
    drawProfile(ctx, compiled.blocks, { x: 0, y: 2, w, h: h - 2, t0: 0, t1: end, maxW }, { colors: theme.colors, zones: backend.zones.power, showAi: true });
  }
</script>

<canvas {@attach canvas2d(draw)}></canvas>

<style>
  canvas {
    display: block;
    width: 100%;
    height: 100%;
  }
</style>
