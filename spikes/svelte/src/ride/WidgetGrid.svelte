<script lang="ts">
  import type { Page } from "@neo/shared";
  import { layout } from "../lib/layout.svelte";
  import { label } from "../lib/format";
  import WidgetView from "./widgets/WidgetView.svelte";
  import { gridDrag, type GridInfo } from "./gridDrag";

  let { page }: { page: Page } = $props();

  let gridEl = $state<HTMLElement>();
  const rows = $derived(layout.rows + (layout.editing ? 1 : 0));
  const grid: GridInfo = { el: () => gridEl, cols: 12, rows: () => rows };

  // Swipe between pages (touch / pen / mouse drag) when not editing.
  let swipeX: number | null = null;
  function down(e: PointerEvent): void {
    if (!layout.editing && e.pointerType !== "mouse") swipeX = e.clientX;
  }
  function up(e: PointerEvent): void {
    if (swipeX === null) return;
    const dx = e.clientX - swipeX;
    swipeX = null;
    if (Math.abs(dx) > 80) layout.go(dx < 0 ? 1 : -1);
  }
</script>

<div class="grid" class:editing={layout.editing} bind:this={gridEl} style:grid-template-rows="repeat({rows}, minmax(0, 1fr))" style:--rows={rows} onpointerdown={down} onpointerup={up} role="presentation">
  {#each page.widgets as w (`${page.id}/${w.id}`)}
    <section
      class="cell"
      class:selected={layout.editing && layout.selectedId === w.id}
      style:grid-column="{w.x + 1} / span {w.w}"
      style:grid-row="{w.y + 1} / span {w.h}"
      data-kind={w.kind}
    >
      <WidgetView widget={w} />
      {#if layout.editing}
        <div class="overlay" {@attach gridDrag(w, "move", { ...grid, onStart: () => (layout.selectedId = w.id) })}>
          <div class="meta">
            <b>{w.kind}</b>
            <span>{w.metrics.map(label).join(" · ") || "—"}</span>
            <span>{w.rateHz} Hz{w.filter?.length ? ` · ${w.filter.map((f) => f.kind).join("→")}` : ""}</span>
          </div>
          <button
            type="button"
            class="rm"
            title="Remove widget"
            onpointerdown={(e) => e.stopPropagation()}
            onclick={() => {
              layout.removeWidget(w.id);
            }}>✕</button
          >
          <div class="resize" title="Resize" {@attach gridDrag(w, "resize", grid)}></div>
        </div>
      {/if}
    </section>
  {/each}
</div>

<style>
  .grid {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: repeat(12, minmax(0, 1fr));
    gap: 8px;
    padding: 8px;
    touch-action: pan-y;
  }
  .grid.editing {
    background-image:
      linear-gradient(to right, var(--grid) 1px, transparent 1px),
      linear-gradient(to bottom, var(--grid) 1px, transparent 1px);
    background-size: calc((100% - 16px) / 12) calc((100% - 16px) / var(--rows, 9));
    background-position: 8px 8px;
  }
  .cell {
    position: relative;
    min-width: 0;
    min-height: 0;
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    box-shadow: var(--shadow);
    overflow: hidden;
  }
  .cell.selected {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .overlay {
    position: absolute;
    inset: 0;
    background: color-mix(in srgb, var(--surface-1) 55%, transparent);
    border: 2px dashed var(--ink-muted);
    border-radius: var(--radius);
    cursor: grab;
    touch-action: none;
    z-index: 5;
  }
  .overlay:active {
    cursor: grabbing;
  }
  .meta {
    position: absolute;
    left: 8px;
    top: 6px;
    display: flex;
    flex-direction: column;
    font-size: 0.72rem;
    background: var(--surface-2);
    border: 1px solid var(--border);
    padding: 0.2rem 0.45rem;
    border-radius: 6px;
    color: var(--ink-secondary);
  }
  .meta b {
    color: var(--ink-primary);
  }
  .rm {
    position: absolute;
    right: 6px;
    top: 6px;
    min-height: 28px;
    padding: 0 0.5rem;
  }
  .resize {
    position: absolute;
    right: 0;
    bottom: 0;
    width: 22px;
    height: 22px;
    cursor: nwse-resize;
    background: linear-gradient(135deg, transparent 50%, var(--accent) 50%);
    border-bottom-right-radius: var(--radius);
    touch-action: none;
  }
</style>
