import { useMetric } from "../../lib/live";
import { fmtValue } from "../../lib/format";
import { WidgetFrame, type WidgetProps } from "./common";
import { info } from "./metrics";

// P03 / P11 — value vs target band (or vs its own floor for W'bal / compliance).
// Re-renders only itself at the widget's rate; the bar is a CSS width.
export function ZoneBar({ widget }: WidgetProps) {
  const [metric, targetMetric] = widget.metrics;
  const m = info(metric ?? "wbal_pct");
  const v = useMetric(m.id, widget.rateHz, widget.filter)?.v ?? null;
  const tv = useMetric(targetMetric, widget.rateHz)?.v ?? null;
  const pctScale = m.unit === "%";
  const max = pctScale ? 100 : Math.max(1, (tv ?? v ?? 1) * 1.5);
  const frac = v === null ? 0 : Math.max(0, Math.min(1, v / max));
  // status only when it's genuinely a state (W'bal running low); always icon + label
  const status = m.id === "wbal_pct" && widget.colorByZone && v !== null ? (v < 25 ? "critical" : v < 50 ? "warning" : "good") : null;
  const statusLabel = status === "critical" ? "▲ empty soon" : status === "warning" ? "! low" : status === "good" ? "✓ ok" : null;
  return (
    <WidgetFrame
      title={m.label}
      aside={
        <span className="zb-value">
          {fmtValue(v, m.unit, m.id)}
          <small>{m.unit}</small>
          {status && <span className={`status-tag ${status}`}>{statusLabel}</span>}
        </span>
      }
      className="zone-bar"
    >
      <div className="zb-track" role="meter" aria-valuemin={0} aria-valuemax={max} aria-valuenow={v ?? 0} aria-label={m.label}>
        {tv !== null && <div className="zb-band" style={{ left: `${((tv * 0.95) / max) * 100}%`, width: `${((tv * 0.1) / max) * 100}%` }} />}
        <div className={`zb-fill ${status ?? ""}`} style={{ width: `${frac * 100}%` }} />
        {m.id === "wbal_pct" && <div className="zb-floor" style={{ left: "25%" }} title="floor 25 %" />}
      </div>
    </WidgetFrame>
  );
}
