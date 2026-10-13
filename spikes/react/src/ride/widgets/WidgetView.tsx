import type { Widget } from "@neo/shared";
import { BigNumber } from "./BigNumber";
import { NumberSparkline } from "./NumberSparkline";
import { ZoneBar } from "./ZoneBar";
import { LogTimeChart } from "./LogTimeChart";
import { HistogramWidget } from "./HistogramWidget";
import { HeatmapWidget } from "./HeatmapWidget";
import { MmpWidget } from "./MmpWidget";
import { WorkoutStrip } from "./WorkoutStrip";
import { HrRecoveryWidget } from "./HrRecoveryWidget";
import { CoachCard } from "./CoachCard";

/** Every widget kind owns its own subscription; nothing above it re-renders on telemetry. */
export function WidgetView({ widget }: { widget: Widget }) {
  switch (widget.kind) {
    case "big_number":
      return <BigNumber widget={widget} />;
    case "number_sparkline":
      return <NumberSparkline widget={widget} />;
    case "zone_bar":
      return <ZoneBar widget={widget} />;
    case "log_time_chart":
      return <LogTimeChart widget={widget} />;
    case "histogram":
      return <HistogramWidget widget={widget} />;
    case "heatmap2d":
      return <HeatmapWidget widget={widget} />;
    case "mmp_curve":
      return <MmpWidget widget={widget} />;
    case "workout_strip":
      return <WorkoutStrip widget={widget} />;
    case "hr_recovery":
      return <HrRecoveryWidget widget={widget} />;
    case "coach_card":
      return <CoachCard />;
  }
}
