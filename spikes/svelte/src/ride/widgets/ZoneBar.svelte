<script lang="ts">
  import type { Widget } from "@neo/shared";
  import { liveMetric } from "../../lib/live.svelte";
  import { fmtValue, label, unit, zonesFor } from "../../lib/format";
  import { theme, seriesColor, zoneColor } from "../../lib/theme.svelte";

  // P03 target-band bar / P11 W'bal gauge: value as a filled bar + marker, target band shaded,
  // zone ramp as a thin labelled strip underneath.
  let { widget }: { widget: Widget } = $props();

  const metric = $derived(widget.metrics[0]);
  const targetMetric = $derived(widget.metrics[1]);
  const value = liveMetric(
    () => metric,
    () => (widget.filter ? { rateHz: widget.rateHz, filter: widget.filter } : { rateHz: widget.rateHz }),
  );
  const target = liveMetric(
    () => targetMetric,
    () => ({ rateHz: 2 }),
  );

  const isPct = $derived(metric ? unit(metric) === "%" : false);
  const zones = $derived(metric && metric !== "wbal_pct" ? zonesFor(metric) : undefined);
  const domain = $derived.by((): [number, number] => {
    if (isPct && metric !== "pct_ftp") return [0, 100];
    const zs = zones;
    if (zs && zs.length) {
      const last = zs.at(-1);
      const prev = zs.at(-2);
      return [0, prev ? prev.max * 1.15 : (last?.max ?? 100)];
    }
    return [0, Math.max(100, (target.v ?? 0) * 1.5)];
  });
  const pos = (v: number) => Math.max(0, Math.min(100, ((v - domain[0]) / (domain[1] - domain[0])) * 100));
  const floor = $derived(metric === "wbal_pct" ? 25 : null);
  const low = $derived(floor !== null && value.v !== null && value.v < floor);
</script>

<div class="tile">
  <div class="head">
    <span class="label">{metric ? label(metric) : "—"}</span>
    <span class="value tabular">{metric ? fmtValue(metric, value.v) : "—"}<small>{metric ? unit(metric) : ""}</small></span>
    {#if targetMetric && target.v !== null}<span class="secondary">target {fmtValue(targetMetric, target.v)}</span>{/if}
    {#if low}<span class="status warning">low — ease off</span>{/if}
  </div>
  <div class="track" role="meter" aria-valuenow={value.v ?? 0} aria-valuemin={domain[0]} aria-valuemax={domain[1]} aria-label={metric ? label(metric) : ""}>
    {#if value.v !== null}
      <div class="fill" style:width="{pos(value.v)}%" style:background={metric ? seriesColor(theme.colors, metric) : "var(--accent)"}></div>
    {/if}
    {#if target.v !== null}
      <div class="band" style:left="{pos(target.v * 0.95)}%" style:width="{pos(target.v * 1.05) - pos(target.v * 0.95)}%"></div>
    {/if}
    {#if floor !== null}
      <div class="floor" style:left="{pos(floor)}%"><span>floor</span></div>
    {/if}
    {#if value.v !== null}<div class="marker" style:left="{pos(value.v)}%"></div>{/if}
  </div>
  {#if zones && widget.colorByZone}
    <div class="zones">
      {#each zones as z, i (z.name)}
        <div class="z" style:left="{pos(z.min)}%" style:width="{pos(z.max) - pos(z.min)}%" style:background={zoneColor(theme.colors, i, zones.length)}>
          <span>{z.name.split(" ")[0]}</span>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .tile {
    height: 100%;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 0.35rem;
    padding: 0.4rem 0.8rem;
  }
  .head {
    display: flex;
    align-items: baseline;
    gap: 0.8rem;
    font-size: 0.8rem;
    color: var(--ink-secondary);
  }
  .label {
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
  .value {
    font-size: 1.6rem;
    font-weight: 650;
    color: var(--ink-primary);
  }
  .value small {
    font-size: 0.5em;
    color: var(--ink-secondary);
    margin-left: 0.15em;
  }
  .track {
    position: relative;
    height: 18px;
    border-radius: 4px;
    background: var(--surface-2);
    overflow: visible;
    flex: 0 0 auto;
  }
  .fill {
    position: absolute;
    inset: 0 auto 0 0;
    border-radius: 4px;
    opacity: 0.85;
    transition: width 0.4s linear;
  }
  .band {
    position: absolute;
    top: -3px;
    bottom: -3px;
    border: 2px dashed var(--target);
    border-radius: 4px;
  }
  .floor {
    position: absolute;
    top: -4px;
    bottom: -4px;
    width: 0;
    border-left: 2px dashed var(--target);
  }
  .floor span {
    position: absolute;
    left: 4px;
    top: -14px;
    font-size: 0.65rem;
    color: var(--ink-muted);
  }
  .marker {
    position: absolute;
    top: -5px;
    bottom: -5px;
    width: 3px;
    margin-left: -1.5px;
    background: var(--ink-primary);
    border-radius: 2px;
    transition: left 0.4s linear;
  }
  .zones {
    position: relative;
    height: 14px;
  }
  .z {
    position: absolute;
    top: 0;
    bottom: 0;
    border-left: 2px solid var(--surface-1);
    font-size: 0.6rem;
    overflow: hidden;
  }
  .z span {
    position: absolute;
    left: 3px;
    top: 0;
    color: var(--ink-primary);
    background: var(--surface-1);
    padding: 0 2px;
    border-radius: 2px;
    line-height: 1.2;
  }
</style>
