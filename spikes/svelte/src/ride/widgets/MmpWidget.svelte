<script lang="ts">
  import type { Widget } from "@neo/shared";
  import { backend } from "../../lib/backend";
  import { poll } from "../../lib/live.svelte";
  import { ride } from "../../lib/ride.svelte";
  import MmpChart from "../../charts/MmpChart.svelte";

  let { widget }: { widget: Widget } = $props();

  // mmp() is cached per ride second: ≤ 0.2 Hz
  const curve = poll(
    () => backend.mmp(),
    () => Math.min(0.2, widget.rateHz),
    () => ride.state === "riding",
  );
</script>

<MmpChart curve={curve.value} />
