import { useMemo, useState } from "react";
import uPlot from "uplot";
import { timeAgoTicks, timeAgoToU, uToTimeAgo, type MetricId } from "@neo/shared";
import { bucketsToTable, type Scale } from "../../charts/history";
import { backend } from "../../backend";
import { usePoll } from "../../lib/live";
import { fmtTimeAgo } from "../../lib/format";
import { useUPlot, type PlotOpts } from "../../charts/useUPlot";
import { alpha, axisStyle } from "../../charts/axes";
import { useChartTheme, type ChartTheme } from "../../theme/theme";
import { WidgetFrame, type WidgetProps } from "./common";
import { info, seriesColor } from "./metrics";

const SPANS = [600, 1800, 7200] as const;

function panelOpts(t: ChartTheme, metric: MetricId, withTarget: boolean, last: boolean, spanS: number, scale: Scale, syncKey: string): PlotOpts {
  const c = seriesColor(t, metric);
  const m = info(metric);
  const ticks = timeAgoTicks(spanS).filter((s) => s === 0 || s >= 5);
  const series: uPlot.Series[] = [
    { label: "ago", value: (_u, v) => (!Number.isFinite(v) ? "—" : fmtTimeAgo(Math.round(uToAgo(1 - v, spanS, scale)))) },
    { label: m.label, stroke: c, width: 2, value: (_u, v) => (!Number.isFinite(v) ? "—" : `${String(Math.round(v))} ${m.unit}`) },
    { label: "max", stroke: "transparent", width: 0, points: { show: false } },
    { label: "min", stroke: "transparent", width: 0, points: { show: false } },
  ];
  if (withTarget) series.push({ label: "target", stroke: t.target, width: 1.5, dash: [6, 4], points: { show: false }, value: (_u, v) => (!Number.isFinite(v) ? "—" : `${String(Math.round(v))} W`) });
  return {
    padding: [6, 8, last ? 0 : 4, 0],
    cursor: { sync: { key: syncKey }, points: { size: 8 }, drag: { x: false, y: false } },
    legend: { show: true, live: true },
    scales: { x: { time: false, range: [0, 1] }, y: { auto: true } },
    series,
    bands: [{ series: [2, 3], fill: alpha(c, t.dark ? 0.28 : 0.22) }],
    axes: [
      {
        ...axisStyle(t),
        show: true,
        size: last ? 26 : 4,
        splits: () => ticks.map((s) => 1 - timeAgoToU(s, spanS, scale)),
        values: (_u, splits) => (last ? splits.map((x) => fmtTimeAgo(Math.round(uToAgo(1 - x, spanS, scale)))) : splits.map(() => "")),
      },
      { ...axisStyle(t), size: 44, label: m.unit, labelSize: 14, labelFont: `11px ${t.font}` },
    ],
  };
}

const uToAgo = (u: number, spanS: number, scale: Scale) => uToTimeAgo(u, spanS, scale);

function Panel(props: { metric: MetricId; withTarget: boolean; last: boolean; spanS: number; scale: Scale; rateHz: number; syncKey: string }) {
  const { metric, withTarget, last, spanS, scale, rateHz, syncKey } = props;
  const theme = useChartTheme();
  const opts = useMemo(() => panelOpts(theme, metric, withTarget, last, spanS, scale, syncKey), [theme, metric, withTarget, last, spanS, scale, syncKey]);
  const { el, setData } = useUPlot(opts);
  usePoll(rateHz, 1, () => {
    const q = { spanS, buckets: 180, scale } as const;
    const main = bucketsToTable(backend.history({ metric, ...q }), spanS, scale);
    if (!withTarget) {
      setData(main);
      return;
    }
    const tgt = bucketsToTable(backend.history({ metric: "target_power", ...q }), spanS, scale);
    setData(uPlot.join([main, [tgt[0], tgt[1]]]));
  }, opts);
  return <div className="plot-panel" ref={el} />;
}

// P04 — log-time stacked panels (one per quantity, shared x and crosshair; never a dual axis).
export function LogTimeChart({ widget }: WidgetProps) {
  const [spanS, setSpanS] = useState(widget.spanS ?? 7200);
  const [scale, setScale] = useState<Scale>(widget.scale ?? "log");
  const hasTarget = widget.metrics.includes("target_power");
  const panels = widget.metrics.filter((m) => m !== "target_power");
  return (
    <WidgetFrame
      title="Timeline"
      className="logtime"
      aside={
        <span className="seg">
          {SPANS.map((s) => (
            <button type="button" key={s} className={s === spanS ? "on" : ""} onClick={() => { setSpanS(s); }}>
              {fmtTimeAgo(s)}
            </button>
          ))}
          <button type="button" className={scale === "fisheye" ? "on" : ""} title="Fisheye: linear recent, compressed past" onClick={() => { setScale(scale === "log" ? "fisheye" : "log"); }}>
            {scale === "log" ? "log" : "fish"}
          </button>
        </span>
      }
    >
      <div className="stack">
        {panels.map((m, i) => (
          <Panel
            key={m}
            metric={m}
            withTarget={hasTarget && m.startsWith("power")}
            last={i === panels.length - 1}
            spanS={spanS}
            scale={scale}
            rateHz={widget.rateHz}
            syncKey={`sync-${widget.id}`}
          />
        ))}
      </div>
    </WidgetFrame>
  );
}
