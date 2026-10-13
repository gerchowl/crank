<script lang="ts">
  import type { CompiledWorkout } from "@neo/shared";
  import { backend } from "../lib/backend";
  import { theme, zoneColor } from "../lib/theme.svelte";
  import { clock } from "../lib/format";

  // Summary from compileWorkout: duration (range), TSS, IF, time-in-zone bars (zone ramp,
  // labelled), validation issues (icon + label). Optional `compare` shows a diff (P24).
  let { compiled, compare }: { compiled: CompiledWorkout; compare?: CompiledWorkout | null | undefined } = $props();

  const zones = backend.zones.power;
  const maxZ = $derived(Math.max(1, ...compiled.timeInZoneS, ...(compare?.timeInZoneS ?? [])));
  const range = $derived(compiled.totalRangeS[0] !== compiled.totalRangeS[1]);
  function delta(a: number, b: number | undefined, fmt: (x: number) => string): string {
    if (b === undefined || Math.abs(a - b) < 1e-6) return "";
    return ` (${a > b ? "+" : "−"}${fmt(Math.abs(a - b))})`;
  }
</script>

<div class="summary">
  <div class="tiles">
    <div>
      <span class="k">Duration</span>
      <b class="tabular">{clock(compiled.totalS)}</b>
      {#if range}<span class="muted tabular">{clock(compiled.totalRangeS[0])}–{clock(compiled.totalRangeS[1])}</span>{/if}
      {#if compare}<span class="secondary">{delta(compiled.totalS, compare.totalS, clock)}</span>{/if}
    </div>
    <div>
      <span class="k">TSS</span><b class="tabular">{Math.round(compiled.tss)}</b>
      {#if compare}<span class="secondary">{delta(compiled.tss, compare.tss, (x) => x.toFixed(0))}</span>{/if}
    </div>
    <div>
      <span class="k">IF</span><b class="tabular">{compiled.if.toFixed(2)}</b>
      {#if compare}<span class="secondary">{delta(compiled.if, compare.if, (x) => x.toFixed(2))}</span>{/if}
    </div>
  </div>
  <div class="zones" role="table" aria-label="Time in zone">
    {#each zones as z, i (z.name)}
      {@const s = compiled.timeInZoneS[i] ?? 0}
      <div class="zrow" role="row">
        <span class="zl" role="cell">{z.name.split(" ")[0]}</span>
        <span class="bar" role="cell">
          <i style:width="{(s / maxZ) * 100}%" style:background={zoneColor(theme.colors, i, zones.length)}></i>
          {#if compare}<u style:width="{((compare.timeInZoneS[i] ?? 0) / maxZ) * 100}%"></u>{/if}
        </span>
        <span class="zt tabular muted" role="cell">{s > 0 ? clock(s) : ""}</span>
      </div>
    {/each}
  </div>
  {#if compiled.issues.length}
    <ul class="issues">
      {#each compiled.issues as iss, i (i)}
        <li><span class="status {iss.severity === 'error' ? 'critical' : 'warning'}">{iss.severity}</span> {iss.message}</li>
      {/each}
    </ul>
  {:else}
    <p class="status good">no validation issues</p>
  {/if}
</div>

<style>
  .summary {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    font-size: 0.85rem;
  }
  .tiles {
    display: flex;
    gap: 1.2rem;
  }
  .tiles div {
    display: flex;
    flex-direction: column;
  }
  .k {
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--ink-secondary);
  }
  .tiles b {
    font-size: 1.4rem;
  }
  .zones {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .zrow {
    display: grid;
    grid-template-columns: 2rem 1fr 3.2rem;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.75rem;
  }
  .bar {
    position: relative;
    height: 12px;
  }
  .bar i {
    position: absolute;
    inset: 0 auto 0 0;
    border-radius: 0 4px 4px 0;
  }
  .bar u {
    position: absolute;
    inset: 0 auto 0 0;
    border: 1.5px dashed var(--ink-muted);
    border-radius: 0 4px 4px 0;
  }
  .zt {
    text-align: right;
  }
  .issues {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
  }
  p {
    margin: 0;
  }
</style>
