<script lang="ts">
  import type { Widget } from "@neo/shared";
  import { backend } from "../../lib/backend";
  import { poll } from "../../lib/live.svelte";
  import { ride } from "../../lib/ride.svelte";
  import { label } from "../../lib/format";
  import HistogramChart from "../../charts/HistogramChart.svelte";

  let { widget }: { widget: Widget } = $props();

  const metric = $derived(widget.metrics[0] ?? "power");
  const DEFAULT_BIN: Partial<Record<string, number>> = { power: 20, hr: 5, cadence: 5, pct_ftp: 5 };
  const BINS = [1, 2, 5, 10, 20, 25, 50];
  let binWidth = $state<number | null>(null);
  const bw = $derived(binWidth ?? DEFAULT_BIN[metric] ?? 10);

  // histogram() is O(ride seconds): ≤ 0.5 Hz
  const hist = poll(
    () => backend.histogram(metric, bw),
    () => Math.min(0.5, widget.rateHz),
    () => ride.state === "riding",
  );
</script>

<HistogramChart hist={hist.value} title="{label(metric)} · time in bin" colorByZone={widget.colorByZone ?? true}>
</HistogramChart>
<select class="bins" value={bw} onchange={(e) => (binWidth = Number(e.currentTarget.value))} aria-label="Bin width">
  {#each BINS as b (b)}<option value={b}>{b}</option>{/each}
</select>

<style>
  .bins {
    position: absolute;
    right: 64px;
    top: 6px;
    font-size: 0.72rem;
    padding: 0 0.2rem;
  }
</style>
