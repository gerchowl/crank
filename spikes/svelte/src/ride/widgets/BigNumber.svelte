<script lang="ts">
  import type { Filter, Widget } from "@neo/shared";
  import { liveMetric } from "../../lib/live.svelte";
  import { fmtValue, label, unit, zoneOf, targetText, endText, dur, isTimeMetric } from "../../lib/format";
  import { ride } from "../../lib/ride.svelte";
  import { findNode } from "@neo/shared";
  import { layout } from "../../lib/layout.svelte";

  let { widget }: { widget: Widget } = $props();

  // P01 "tap → cycle smoothing": a session-local override of the widget's filter chain.
  const SMOOTHING: { name: string; filter: Filter[] | undefined }[] = [
    { name: "as set", filter: undefined },
    { name: "raw", filter: [] },
    { name: "EMA 2 s", filter: [{ kind: "ema", tauS: 2 }] },
    { name: "SMA 10 s", filter: [{ kind: "sma", windowS: 10 }] },
  ];
  let smoothing = $state(0);

  const metric = $derived(widget.metrics[0]);
  const targetMetric = $derived(widget.metrics[1]);
  const filter = $derived(SMOOTHING[smoothing]?.filter ?? widget.filter);

  const value = liveMetric(
    () => metric,
    () => (filter ? { rateHz: widget.rateHz, filter } : { rateHz: widget.rateHz }),
  );
  const target = liveMetric(
    () => targetMetric,
    () => ({ rateHz: Math.min(widget.rateHz, 2) }),
  );

  const zone = $derived(widget.colorByZone && metric ? zoneOf(metric, value.v) : null);
  const delta = $derived.by(() => {
    const v = value.v;
    const t = target.v;
    if (v === null || t === null || t === 0) return null;
    return ((v - t) / t) * 100;
  });
  const deltaClass = $derived(delta === null ? "" : Math.abs(delta) < 3 ? "mid" : delta > 0 ? "pos" : "neg");

  // The countdown tile (block_remaining) carries block x/n and the next-block preview.
  const isCountdown = $derived(metric === "block_remaining");
  const nextText = $derived.by(() => {
    const nb = ride.nextBlock;
    const cur = ride.current;
    if (!nb || !cur) return "last block";
    const node = findNode(cur.workout.nodes, nb.nodeId);
    const what = node?.type === "step" ? `${endText(node.end)} @ ${targetText(node.target)}` : dur(nb.durS);
    return `next: ${what} · ${Math.round(nb.powerFrom)} W`;
  });
</script>

<button
  type="button"
  class="tile"
  onclick={() => {
    if (!layout.editing) smoothing = (smoothing + 1) % SMOOTHING.length;
  }}
  title="Tap to cycle smoothing ({SMOOTHING[smoothing]?.name})"
>
  <span class="head">
    <span class="label">{metric ? label(metric) : "—"}</span>
    {#if smoothing > 0}<span class="chip muted">{SMOOTHING[smoothing]?.name}</span>{/if}
    {#if zone}
      <span class="zone"><i style:background={`var(--zone-${Math.round((zone.index * 6) / Math.max(1, zone.n - 1)) + 1})`}></i>{zone.name}</span>
    {/if}
  </span>
  <span class="value tabular">
    {metric ? fmtValue(metric, value.v) : "—"}<small>{metric && !isTimeMetric(metric) ? unit(metric) : ""}</small>
  </span>
  {#if targetMetric && target.v !== null}
    <span class="sub">
      <span class="secondary">{label(targetMetric).replace("Target cadence", "target").replace("Target", "target")} {fmtValue(targetMetric, target.v)} {unit(targetMetric)}</span>
      {#if delta !== null}
        <span class="delta {deltaClass}">{delta > 0 ? "+" : ""}{delta.toFixed(0)}%</span>
      {/if}
    </span>
  {:else if targetMetric}
    <span class="sub muted">no target</span>
  {:else if isCountdown}
    <span class="sub secondary">block {ride.blockIndex + 1}/{ride.blocks.length} · {nextText}</span>
  {/if}
</button>

<style>
  .tile {
    all: unset;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    width: 100%;
    height: 100%;
    padding: 0.6rem 0.8rem;
    container-type: size;
    cursor: pointer;
    overflow: hidden;
  }
  .tile:focus-visible {
    outline: 2px solid var(--focus);
  }
  .head {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.8rem;
    letter-spacing: 0.02em;
    font-weight: 600;
    color: var(--ink-secondary);
  }
  .label {
    flex: 1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .chip {
    text-transform: none;
    letter-spacing: 0;
  }
  .zone {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    font-weight: 600;
    color: var(--ink-primary);
  }
  .zone i {
    width: 0.8rem;
    height: 0.8rem;
    border-radius: 3px;
  }
  .value {
    font-weight: 650;
    line-height: 0.95;
    /* ≥ 64 px on the ride screen; scale with the tile */
    font-size: clamp(2rem, min(48cqh, 26cqw), 9rem);
    white-space: nowrap;
  }
  @container (min-height: 140px) {
    .value {
      font-size: clamp(64px, min(52cqh, 26cqw), 9rem);
    }
  }
  .value small {
    font-size: 0.3em;
    font-weight: 500;
    color: var(--ink-secondary);
    margin-left: 0.2em;
  }
  .sub {
    display: flex;
    gap: 0.6rem;
    align-items: center;
    font-size: 0.85rem;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .delta {
    font-weight: 600;
    padding: 0.05rem 0.4rem;
    border-radius: 999px;
    color: var(--ink-primary);
    border: 2px solid var(--div-mid);
  }
  .delta.pos {
    border-color: var(--div-pos);
  }
  .delta.neg {
    border-color: var(--div-neg);
  }
</style>
