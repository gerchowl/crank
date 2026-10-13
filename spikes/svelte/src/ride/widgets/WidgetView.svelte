<script lang="ts">
  import type { Component } from "svelte";
  import type { Widget, WidgetKind } from "@neo/shared";
  import BigNumber from "./BigNumber.svelte";
  import NumberSparkline from "./NumberSparkline.svelte";
  import ZoneBar from "./ZoneBar.svelte";
  import LogTimeChart from "./LogTimeChart.svelte";
  import HistogramWidget from "./HistogramWidget.svelte";
  import HeatmapWidget from "./HeatmapWidget.svelte";
  import MmpWidget from "./MmpWidget.svelte";
  import WorkoutStrip from "./WorkoutStrip.svelte";
  import HrRecovery from "./HrRecovery.svelte";
  import CoachCard from "./CoachCard.svelte";

  // Kind → component registry. Components are values in Svelte 5, so dispatch is a lookup;
  // the Record type makes a missing kind a compile error.
  const KINDS: Record<WidgetKind, Component<{ widget: Widget }>> = {
    big_number: BigNumber,
    number_sparkline: NumberSparkline,
    zone_bar: ZoneBar,
    log_time_chart: LogTimeChart,
    histogram: HistogramWidget,
    heatmap2d: HeatmapWidget,
    mmp_curve: MmpWidget,
    workout_strip: WorkoutStrip,
    hr_recovery: HrRecovery,
    coach_card: CoachCard,
  };

  let { widget }: { widget: Widget } = $props();
  const View = $derived(KINDS[widget.kind]);
</script>

<View {widget} />
