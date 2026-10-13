import { useRef } from "react";
import { useMetricCallback, useRideStatus, useRideWorkout } from "../../lib/live";
import { useCanvas } from "../../charts/useCanvas";
import { paintProfile } from "../../charts/profile";
import { useChartTheme } from "../../theme/theme";
import { fmtDur } from "../../lib/format";
import { WidgetFrame, type WidgetProps } from "./common";

// P13 strip — full profile (or an upcoming window when spanS is set), "you are here", AI-changed blocks outlined.
export function WorkoutStrip({ widget }: WidgetProps) {
  const cur = useRideWorkout();
  const theme = useChartTheme();
  const idx = useRideStatus((s) => s?.blockIndex ?? 0);
  const now = useRef(0);
  const blocks = cur?.compiled.blocks ?? [];
  const span = widget.spanS;
  const { ref, redraw } = useCanvas((ctx, w, h) => {
    const t = now.current;
    paintProfile(ctx, w, h, blocks, theme, span ? { from: t - span * 0.1, to: t + span * 0.9, nowS: t } : { nowS: t });
  }, [blocks, theme, span]);
  useMetricCallback("elapsed", widget.rateHz, undefined, (s) => {
    if (s.v === null) return;
    now.current = s.v;
    redraw();
  });
  const next = blocks[idx + 1];
  const aiCount = blocks.filter((b) => b.aiChanged).length;
  return (
    <WidgetFrame
      title={span ? "Next" : "Workout"}
      aside={
        <span className="muted small">
          {next ? `next: ${fmtDur(next.durS)} @ ${Math.round(next.powerFrom)} W (${next.phase})` : "last block"}
          {aiCount > 0 && <span className="ai-tag"> ✦ {aiCount} AI-changed</span>}
        </span>
      }
      className="strip"
    >
      <canvas ref={ref} className="fill-canvas" role="img" aria-label="Workout profile with current position" />
    </WidgetFrame>
  );
}
