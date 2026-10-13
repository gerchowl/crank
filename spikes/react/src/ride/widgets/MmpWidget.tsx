import { useMemo, useState } from "react";
import type uPlot from "uplot";
import type { MmpCurve } from "@neo/shared";
import { backend } from "../../backend";
import { usePoll } from "../../lib/live";
import { fmtDur } from "../../lib/format";
import { useUPlot, type PlotOpts } from "../../charts/useUPlot";
import { axisStyle } from "../../charts/axes";
import { useChartTheme, type ChartTheme } from "../../theme/theme";
import { WidgetFrame, type WidgetProps } from "./common";
import { toMmpData } from "../../charts/mmp";
import { TableToggle, TableView } from "./TableView";

const TICKS = [1, 5, 15, 60, 300, 1200, 3600];

function opts(t: ChartTheme): PlotOpts {
  const w = (_u: uPlot, v: number) => (Number.isFinite(v) ? `${String(Math.round(v))} W` : "—");
  return {
    padding: [8, 10, 0, 0],
    legend: { show: true, live: true },
    cursor: { points: { size: 8 }, drag: { x: false, y: false } },
    // uPlot declares Distr as an ambient const enum (not importable under isolatedModules); 3 = log
    // eslint-disable-next-line @typescript-eslint/no-unsafe-enum-assignment
    scales: { x: { time: false, distr: 3, log: 10 }, y: { range: (_u, _min, max) => [0, Math.ceil((max * 1.05) / 100) * 100] } },
    series: [
      { label: "duration", value: (_u, v) => (Number.isFinite(v) ? fmtDur(v) : "—") },
      { label: "today", stroke: t.series.power, width: 2, value: w, points: { show: false } },
      { label: "90 d", stroke: t.ink2, width: 1.5, dash: [6, 4], value: w, points: { show: false } },
      { label: "all-time", stroke: t.muted, width: 1.5, dash: [2, 3], value: w, points: { show: false } },
    ],
    axes: [
      { ...axisStyle(t), size: 24, splits: () => TICKS, values: (_u, s) => s.map((v) => (Number.isFinite(v) && v > 0 ? fmtDur(v) : "")) },
      { ...axisStyle(t), size: 40, label: "W", labelSize: 14, labelFont: `11px ${t.font}` },
    ],
  };
}

// P09 — mean-maximal power, log x; today vs 90 d vs all-time (no model fit in v0). Polled ≤ 0.2 Hz.
export function MmpWidget({ widget, data: given }: WidgetProps & { data?: MmpCurve }) {
  const theme = useChartTheme();
  const o = useMemo(() => opts(theme), [theme]);
  const { el, setData } = useUPlot(o);
  const [table, setTable] = useState<MmpCurve | null>(null);
  const [showTable, setShowTable] = useState(false);
  usePoll(widget.rateHz, 0.2, () => {
    const c = given ?? backend.mmp();
    setData(toMmpData(c));
    if (showTable) setTable(c);
  }, showTable);
  return (
    <WidgetFrame title="Power-duration (MMP)" className="mmp" aside={<TableToggle on={showTable} onChange={setShowTable} />}>
      {showTable && table ? (
        <TableView
          head={["duration", "today", "90 d", "all-time", "% of best"]}
          rows={table.durationsS.map((d, i) => {
            const td = table.today[i] ?? 0;
            const best = table.allTime[i] ?? 1;
            return [fmtDur(d), td ? Math.round(td) : "—", Math.round(table.best90d[i] ?? 0), Math.round(best), td ? `${String(Math.round((td / best) * 100))}%` : "—"];
          })}
        />
      ) : null}
      <div className="plot-panel" ref={el} hidden={showTable} />
    </WidgetFrame>
  );
}
