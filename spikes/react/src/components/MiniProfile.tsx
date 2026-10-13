import type { FlatBlock } from "@neo/shared";
import { useCanvas } from "../charts/useCanvas";
import { paintProfile } from "../charts/profile";
import { useChartTheme } from "../theme/theme";

/** P13 mini (read-only) — used by the check-in proposal and the review header. */
export function MiniProfile({ blocks, height = 90, label }: { blocks: readonly FlatBlock[]; height?: number; label: string }) {
  const theme = useChartTheme();
  const { ref } = useCanvas((ctx, w, h) => {
    paintProfile(ctx, w, h, blocks, theme);
  }, [blocks, theme]);
  return <canvas ref={ref} style={{ width: "100%", height }} role="img" aria-label={label} />;
}
