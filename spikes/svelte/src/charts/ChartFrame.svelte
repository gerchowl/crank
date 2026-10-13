<script lang="ts">
  import type { Snippet } from "svelte";

  // Common analytics-chart chrome: title, optional controls, chart ⇄ table toggle (VIEWS §0:
  // every analytics plot has a table view). Content and table are snippets.
  let {
    title,
    controls,
    chart,
    table,
    legend,
  }: {
    title: string;
    controls?: Snippet | undefined;
    chart: Snippet;
    table?: Snippet | undefined;
    legend?: Snippet | undefined;
  } = $props();

  let showTable = $state(false);
</script>

<div class="frame">
  <div class="head">
    <span class="title">{title}</span>
    {#if legend}<span class="legend">{@render legend()}</span>{/if}
    <span class="spacer"></span>
    {@render controls?.()}
    {#if table}
      <button type="button" class="ghost small" aria-pressed={showTable} onclick={() => (showTable = !showTable)} title="Toggle table view">
        {showTable ? "chart" : "table"}
      </button>
    {/if}
  </div>
  <div class="body">
    {#if showTable && table}
      <div class="table">{@render table()}</div>
    {:else}
      {@render chart()}
    {/if}
  </div>
</div>

<style>
  .frame {
    height: 100%;
    display: flex;
    flex-direction: column;
    padding: 0.4rem 0.6rem;
    min-height: 0;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.75rem;
    min-height: 26px;
  }
  .title {
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--ink-secondary);
    font-size: 0.75rem;
  }
  .legend {
    display: flex;
    gap: 0.7rem;
    color: var(--ink-secondary);
  }
  .spacer {
    flex: 1;
  }
  .head :global(button.small) {
    min-height: 22px;
    padding: 0 0.45rem;
    font-size: 0.72rem;
  }
  .body {
    flex: 1;
    min-height: 0;
    position: relative;
  }
  .table {
    position: absolute;
    inset: 0;
    overflow: auto;
  }
</style>
