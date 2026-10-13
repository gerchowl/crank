<script lang="ts">
  import type { Widget } from "@neo/shared";
  import { backend } from "../../lib/backend";
  import { poll } from "../../lib/live.svelte";
  import { ride } from "../../lib/ride.svelte";
  import { clock } from "../../lib/format";
  import HrRecoveryChart from "../../charts/HrRecoveryChart.svelte";

  // P12 live: polls hrRecovery() at the widget rate (1–2 Hz) only during recovery blocks;
  // otherwise slowly, showing the last block's final fit.
  let { widget }: { widget: Widget } = $props();

  const rec = poll(
    () => backend.hrRecovery(),
    () => (ride.phase === "recovery" ? Math.max(1, Math.min(2, widget.rateHz)) : 0.2),
    () => ride.state === "riding",
  );
  const s = $derived(rec.value);
  const lastHr = $derived(s?.observed.findLast((o) => o.v !== null)?.v ?? null);
  const slower = $derived(s?.fit ? s.fit.tauS > s.fit.usualTauS * 1.1 : false);
</script>

<div class="rec">
  <div class="head">
    <span class="title">HR recovery</span>
    {#if s?.active}<span class="status good">live</span>{:else if s && s.observed.length}<span class="muted">last block</span>{/if}
    <span class="legend">
      <span><i class="dot"></i>observed</span>
      <span><i class="line"></i>fit</span>
      <span><i class="dots"></i>prediction · 90 % band</span>
      {#if s?.targetBpm}<span><i class="dash"></i>target {s.targetBpm} bpm</span>{/if}
    </span>
  </div>
  <div class="chart">
    {#if s && s.observed.length}
      <HrRecoveryChart state={s} />
    {:else}
      <p class="muted empty">Waiting for an HR-guided recovery block (hr&lt;…).</p>
    {/if}
  </div>
  <div class="foot">
    <div class="ready">
      <span class="k">Ready in</span>
      {#if s?.readyInS && !s.unreachable}
        <span class="big tabular">≈ {clock(s.readyInS.mean)}</span>
        <span class="secondary tabular">({clock(s.readyInS.lo)}–{clock(s.readyInS.hi)})</span>
      {:else if s?.unreachable}
        <span class="big">—</span>
      {:else}
        <span class="big muted">…</span>
      {/if}
    </div>
    <div class="kv">
      <span class="k">HR</span><span class="tabular v">{lastHr === null ? "—" : Math.round(lastHr)}</span>
    </div>
    <div class="kv">
      <span class="k tau">τ</span>
      <span class="tabular v">{s?.fit ? `${Math.round(s.fit.tauS)} s` : "—"}</span>
      <span class="secondary">usual {s?.fit ? `${Math.round(s.fit.usualTauS)} s` : "—"}</span>
      {#if slower}<span class="status warning">slower than usual</span>{/if}
    </div>
    {#if s?.unreachable}
      <div class="status serious unreach">Target unreachable — HR settling above {s.targetBpm} bpm{s.fit ? ` (≈${Math.round(s.fit.hrInf)})` : ""}</div>
    {/if}
  </div>
</div>

<style>
  .rec {
    height: 100%;
    display: flex;
    flex-direction: column;
    padding: 0.5rem 0.8rem;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    font-size: 0.75rem;
    flex-wrap: wrap;
  }
  .title {
    letter-spacing: 0.02em;
    font-weight: 600;
    color: var(--ink-secondary);
  }
  .legend {
    display: flex;
    gap: 0.8rem;
    margin-left: auto;
    color: var(--ink-secondary);
  }
  .legend i {
    display: inline-block;
    width: 14px;
    height: 0;
    margin-right: 4px;
    vertical-align: middle;
  }
  .legend .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--series-hr);
  }
  .legend .line {
    border-top: 2px solid var(--series-hr);
  }
  .legend .dots {
    border-top: 2px dotted var(--series-hr);
  }
  .legend .dash {
    border-top: 2px dashed var(--target);
  }
  .chart {
    flex: 1;
    position: relative;
    min-height: 0;
  }
  .empty {
    padding: 2rem;
  }
  .foot {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0.4rem 2rem;
    padding-top: 0.4rem;
  }
  .k {
    text-transform: uppercase;
    font-size: 0.75rem;
    letter-spacing: 0.06em;
    color: var(--ink-secondary);
    margin-right: 0.5rem;
  }
  .k.tau {
    text-transform: none;
    font-size: 1rem;
  }
  .ready .big {
    font-size: clamp(2rem, 4.5vw, 4.5rem);
    font-weight: 650;
  }
  .kv .v {
    font-size: 1.8rem;
    font-weight: 600;
    margin-right: 0.4rem;
  }
  .unreach {
    flex-basis: 100%;
    font-weight: 600;
    color: var(--ink-primary);
  }
</style>
