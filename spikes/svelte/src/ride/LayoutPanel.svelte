<script lang="ts">
  import type { Filter, MetricId, WidgetKind } from "@neo/shared";
  import { layout, KIND_DEFAULTS, type DeviceId } from "../lib/layout.svelte";
  import { backend } from "../lib/backend";
  import { metricInfo } from "../lib/format";

  // V9 layout editor side panel: page + catalog + per-widget settings. Every control binds
  // straight into the layout proxy, so the widget under edit re-subscribes live.
  const KINDS = Object.keys(KIND_DEFAULTS) as WidgetKind[];
  const METRICS = backend.metrics.map((m) => m.id);
  const FILTER_DEFAULTS: Record<Filter["kind"], Filter> = {
    sma: { kind: "sma", windowS: 3 },
    ema: { kind: "ema", tauS: 2 },
    median: { kind: "median", n: 5 },
    one_euro: { kind: "one_euro", minCutoff: 1, beta: 0.01 },
    hold: { kind: "hold" },
  };
  const DEVICES: DeviceId[] = ["laptop", "tablet", "tv"];
  const SPANS = [60, 300, 600, 900, 1800, 3600, 7200];
  const CHART_KINDS = new Set<WidgetKind>(["log_time_chart"]);

  const w = $derived(layout.selected);
  // rate slider is logarithmic: 0.1 … 10 Hz
  const rateToSlider = (r: number) => Math.log10(r);
  const sliderToRate = (s: number) => Math.round(10 ** s * 10) / 10;
</script>

<aside class="panel">
  <header>
    <h2>Layout</h2>
    <select value={layout.device} onchange={(e) => layout.setDevice(e.currentTarget.value as DeviceId)} aria-label="Device layout">
      {#each DEVICES as d (d)}<option value={d}>{d}</option>{/each}
    </select>
  </header>

  {#if layout.page}
    <label class="field">Page name <input bind:value={layout.page.name} /></label>
    <div class="phases">
      auto on:
      {#each ["warmup", "work", "recovery", "cooldown", "free"] as const as ph (ph)}
        <label>
          <input
            type="checkbox"
            checked={layout.page.autoPhase?.includes(ph) ?? false}
            onchange={(e) => {
              const p = layout.page;
              if (!p) return;
              const rest = (p.autoPhase ?? []).filter((x) => x !== ph);
              p.autoPhase = e.currentTarget.checked ? [...rest, ph] : rest;
            }}
          />{ph}
        </label>
      {/each}
    </div>
  {/if}
  <button type="button" onclick={() => layout.addPage()}>+ Page</button>

  <h3>Add widget</h3>
  <div class="catalog">
    {#each KINDS as k (k)}
      <button type="button" onclick={() => layout.addWidget(k)}>{k.replace("_", " ")}</button>
    {/each}
  </div>

  {#if w}
    <h3>Widget <span class="muted">{w.id}</span></h3>
    <label class="field">
      Kind
      <select
        value={w.kind}
        onchange={(e) => {
          const k = e.currentTarget.value as WidgetKind;
          w.kind = k;
          w.metrics = [...KIND_DEFAULTS[k].metrics];
        }}
      >
        {#each KINDS as k (k)}<option value={k}>{k}</option>{/each}
      </select>
    </label>

    <div class="field">
      Metrics
      {#each w.metrics as _m, i (i)}
        <div class="metric">
          <select bind:value={w.metrics[i]}>
            {#each METRICS as id (id)}<option value={id}>{metricInfo.get(id)?.label ?? id}</option>{/each}
          </select>
          <button type="button" class="ghost" onclick={() => w.metrics.splice(i, 1)} title="Remove metric">✕</button>
        </div>
      {/each}
      {#if CHART_KINDS.has(w.kind) || w.metrics.length < 2}
        <button type="button" class="ghost" onclick={() => w.metrics.push("cadence" satisfies MetricId)}>+ metric</button>
      {/if}
    </div>

    <label class="field">
      Display rate <b class="tabular">{w.rateHz} Hz</b>
      <input
        type="range"
        min="-1"
        max="1"
        step="0.05"
        value={rateToSlider(w.rateHz)}
        oninput={(e) => {
          w.rateHz = sliderToRate(Number(e.currentTarget.value));
        }}
      />
      {#if w.metrics[0]}
        {@const src = metricInfo.get(w.metrics[0])?.sourceHz ?? 1}
        <span class="muted hint">source {src} Hz{w.rateHz > src ? " — higher display rates add nothing at 1×" : ""}</span>
      {/if}
    </label>

    <div class="field">
      Filter chain
      {#each w.filter ?? [] as f, i (i)}
        <div class="filter">
          <select
            value={f.kind}
            onchange={(e) => {
              if (w.filter) w.filter[i] = structuredClone(FILTER_DEFAULTS[e.currentTarget.value as Filter["kind"]]);
            }}
          >
            {#each Object.keys(FILTER_DEFAULTS) as k (k)}<option value={k}>{k}</option>{/each}
          </select>
          {#if f.kind === "sma"}
            <input type="number" min="0.5" step="0.5" bind:value={f.windowS} title="window (s)" /> s
          {:else if f.kind === "ema"}
            τ <input type="number" min="0.1" step="0.5" bind:value={f.tauS} title="time constant (s)" /> s
          {:else if f.kind === "median"}
            n <input type="number" min="1" step="1" bind:value={f.n} />
          {:else if f.kind === "one_euro"}
            fc <input type="number" min="0.01" step="0.1" bind:value={f.minCutoff} />
            β <input type="number" min="0" step="0.005" bind:value={f.beta} />
          {/if}
          <button type="button" class="ghost" onclick={() => w.filter?.splice(i, 1)} title="Remove filter">✕</button>
        </div>
      {/each}
      <button
        type="button"
        class="ghost"
        onclick={() => {
          (w.filter ??= []).push({ kind: "ema", tauS: 2 });
        }}>+ filter</button
      >
    </div>

    <label class="check"><input type="checkbox" checked={w.colorByZone ?? false} onchange={(e) => (w.colorByZone = e.currentTarget.checked)} /> colour by zone</label>

    {#if w.kind === "log_time_chart" || w.kind === "number_sparkline" || w.kind === "workout_strip"}
      <label class="field">
        Chart span
        <select
          value={w.spanS ?? 0}
          onchange={(e) => {
            const v = Number(e.currentTarget.value);
            if (v > 0) w.spanS = v;
            else delete w.spanS;
          }}
        >
          {#if w.kind === "workout_strip"}<option value={0}>whole workout</option>{/if}
          {#each SPANS as s (s)}<option value={s}>{s >= 3600 ? `${s / 3600} h` : `${s / 60} min`}</option>{/each}
        </select>
      </label>
    {/if}
    {#if w.kind === "log_time_chart"}
      <label class="field">
        Time scale
        <select value={w.scale ?? "log"} onchange={(e) => (w.scale = e.currentTarget.value === "fisheye" ? "fisheye" : "log")}>
          <option value="log">log (time-ago)</option>
          <option value="fisheye">fisheye</option>
        </select>
      </label>
    {/if}
    <div class="geom muted tabular">x {w.x} · y {w.y} · {w.w}×{w.h}</div>
  {:else}
    <p class="muted">Drag a widget to move it, drag its corner to resize, click it for settings.</p>
  {/if}

  <footer>
    <button type="button" class="primary" onclick={() => layout.save()}>Save layout</button>
    <button type="button" onclick={() => layout.cancelEdit()}>Cancel</button>
    <button type="button" class="ghost" onclick={() => layout.reset()}>Reset to default</button>
  </footer>
</aside>

<style>
  .panel {
    width: 320px;
    flex: 0 0 320px;
    overflow-y: auto;
    padding: 0.8rem;
    background: var(--surface-1);
    border-left: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    font-size: 0.85rem;
  }
  header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  h2,
  h3 {
    margin: 0.3rem 0 0;
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--ink-secondary);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }
  .phases {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    font-size: 0.75rem;
    color: var(--ink-secondary);
  }
  .catalog {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 0.3rem;
  }
  .catalog button {
    font-size: 0.75rem;
    padding: 0.25rem;
    min-height: 30px;
  }
  .metric,
  .filter {
    display: flex;
    align-items: center;
    gap: 0.3rem;
  }
  .metric select {
    flex: 1;
  }
  .filter input {
    width: 4.2rem;
  }
  .hint {
    font-size: 0.72rem;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }
  footer {
    margin-top: auto;
    display: flex;
    gap: 0.4rem;
    flex-wrap: wrap;
    padding-top: 0.6rem;
    border-top: 1px solid var(--border);
  }
</style>
