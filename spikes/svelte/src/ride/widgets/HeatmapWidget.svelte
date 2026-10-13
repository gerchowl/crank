<script lang="ts">
  import type { MetricId, Widget } from "@neo/shared";
  import { backend } from "../../lib/backend";
  import { poll } from "../../lib/live.svelte";
  import { ride } from "../../lib/ride.svelte";
  import { label } from "../../lib/format";
  import HeatmapChart from "../../charts/HeatmapChart.svelte";

  let { widget }: { widget: Widget } = $props();

  const BIN: Partial<Record<MetricId, number>> = { power: 20, hr: 4, cadence: 4 };
  const x = $derived(widget.metrics[0] ?? "power");
  const y = $derived(widget.metrics[1] ?? "hr");

  // heatmap() is O(ride seconds): ≤ 0.5 Hz
  const map = poll(
    () => backend.heatmap(x, y, BIN[x] ?? 10, BIN[y] ?? 5),
    () => Math.min(0.5, widget.rateHz),
    () => ride.state === "riding",
  );
</script>

<HeatmapChart map={map.value} title="{label(x)} × {label(y)}" />
