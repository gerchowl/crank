import { useState } from "react";
import type { HrRecoveryState } from "@neo/shared";
import { backend } from "../../backend";
import { usePoll, useRideStatus } from "../../lib/live";
import { mmss } from "../../lib/format";
import { useCanvas } from "../../charts/useCanvas";
import { paintRecovery } from "../../charts/recovery";
import { useChartTheme } from "../../theme/theme";
import { WidgetFrame, type WidgetProps } from "./common";

// P12 — live HR recovery: fit, prediction band, target, ready-in, τ vs usual, unreachable warning.
export function HrRecoveryWidget({ widget }: WidgetProps) {
  const inRecovery = useRideStatus((s) => s?.phase === "recovery");
  const [rec, setRec] = useState<HrRecoveryState | null>(null);
  const theme = useChartTheme();
  usePoll(inRecovery ? Math.max(1, widget.rateHz) : 0.2, 2, () => {
    setRec(backend.hrRecovery());
  }, inRecovery);
  const { ref } = useCanvas((ctx, w, h) => {
    if (rec && rec.observed.length) paintRecovery(ctx, w, h, rec, theme);
  }, [rec, theme]);

  const f = rec?.fit;
  const slower = f ? f.tauS > f.usualTauS * 1.1 : false;
  const has = !!rec && rec.observed.length > 0;
  return (
    <WidgetFrame title="HR recovery" className="hrrec" aside={has && !rec.active ? <span className="muted small">last recovery</span> : null}>
      {!has ? (
        <p className="muted pad">No HR-guided recovery yet. It appears here during an <code>hr&lt;</code> block.</p>
      ) : (
        <div className="hrrec-grid">
          <canvas ref={ref} className="fill-canvas" role="img" aria-label="HR recovery fit with prediction band" />
          <div className="hrrec-facts">
            <div className="ready">
              <span className="label">{rec.active ? "Ready in" : "Recovered in"}</span>
              <span className="mid-value">
                {rec.readyInS ? `≈ ${mmss(rec.readyInS.mean)}` : "—"}
              </span>
              {rec.readyInS && (
                <span className="muted">
                  ({mmss(rec.readyInS.lo)}–{mmss(rec.readyInS.hi)})
                </span>
              )}
            </div>
            {f && (
              <div className="tau">
                τ <b>{Math.round(f.tauS)} s</b> <span className="muted">(usual {Math.round(f.usualTauS)} s)</span>{" "}
                {slower ? <span className="status-tag warning">! slower</span> : <span className="status-tag good">✓ normal</span>}
              </div>
            )}
            {rec.unreachable && (
              <div className="status-tag serious block">▲ Target unreachable: HR levelling off at {f ? Math.round(f.hrInf) : "?"} bpm</div>
            )}
          </div>
        </div>
      )}
    </WidgetFrame>
  );
}
