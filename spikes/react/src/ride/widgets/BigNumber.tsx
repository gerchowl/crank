import { useState } from "react";
import type { Filter } from "@neo/shared";
import { useMetric, useRideStatus, useRideWorkout } from "../../lib/live";
import { fmtValue } from "../../lib/format";
import { useChartTheme } from "../../theme/theme";
import { WidgetFrame, ZoneChip, type WidgetProps } from "./common";
import { info, zoneOf } from "./metrics";

// P01 — tap cycles smoothing (overrides the widget's filter chain locally).
const SMOOTHING: { label: string; filter: Filter[] | undefined }[] = [
  { label: "", filter: undefined },
  { label: "EMA 2s", filter: [{ kind: "ema", tauS: 2 }] },
  { label: "SMA 10s", filter: [{ kind: "sma", windowS: 10 }] },
  { label: "1€", filter: [{ kind: "one_euro", minCutoff: 0.3, beta: 0.02 }] },
];

export function BigNumber({ widget }: WidgetProps) {
  const [smoothIdx, setSmoothIdx] = useState(0);
  const [metric, targetMetric] = widget.metrics;
  const filter = smoothIdx === 0 ? widget.filter : SMOOTHING[smoothIdx]?.filter;
  const m = info(metric ?? "power");
  const sample = useMetric(m.id, widget.rateHz, filter);
  const target = useMetric(targetMetric, widget.rateHz);
  const theme = useChartTheme();
  const v = sample?.v ?? null;
  const tv = target?.v ?? null;
  const hit = widget.colorByZone ? zoneOf(m.id, v) : null;
  const isPowerTarget = targetMetric === "target_power";
  const delta = v !== null && tv !== null && tv > 0 ? (isPowerTarget ? ((v - tv) / tv) * 100 : v - tv) : null;
  const deltaTone = delta === null ? "" : Math.abs(delta) < (isPowerTarget ? 3 : 2) ? "mid" : delta > 0 ? "pos" : "neg";
  const smoothLabel = SMOOTHING[smoothIdx]?.label ?? "";

  return (
    <WidgetFrame
      title={m.label}
      aside={smoothLabel ? <span className="muted small">{smoothLabel}</span> : null}
      className="big-number"
    >
      <button
        type="button"
        className="big-number-tap"
        title="Tap to cycle smoothing"
        onClick={() => {
          setSmoothIdx((i) => (i + 1) % SMOOTHING.length);
        }}
      >
        <span className="big-value" data-testid={`value-${widget.id}`}>
          {fmtValue(v, m.unit, m.id)}
          {m.unit && m.unit !== "s" ? <small>{m.unit}</small> : null}
        </span>
        <span className="big-meta">
          <ZoneChip hit={hit} theme={theme} />
          {targetMetric && tv !== null && (
            <span className="muted">
              {targetMetric === "target_cadence" ? "target " : "→ "}
              {fmtValue(tv, info(targetMetric).unit, targetMetric)}
              {info(targetMetric).unit ? ` ${info(targetMetric).unit}` : ""}
            </span>
          )}
          {delta !== null && (
            <span className={`delta-chip ${deltaTone}`}>
              {delta > 0 ? "+" : ""}
              {isPowerTarget ? `${Math.round(delta)}%` : Math.round(delta)}
            </span>
          )}
          {m.id === "block_remaining" && <BlockOf />}
        </span>
      </button>
    </WidgetFrame>
  );
}

/** "block x/n · next: …" for the interval countdown (status changes only on block change). */
function BlockOf() {
  const idx = useRideStatus((s) => s?.blockIndex ?? 0);
  const cur = useRideWorkout();
  const blocks = cur?.compiled.blocks ?? [];
  const next = blocks[idx + 1];
  return (
    <span className="muted">
      block {Math.min(idx + 1, blocks.length)}/{blocks.length}
      {next ? ` · next ${Math.round(next.durS / 60) >= 1 ? `${Math.round(next.durS / 60)}′` : `${Math.round(next.durS)}″`} @ ${Math.round(next.powerFrom)} W` : ""}
    </span>
  );
}
