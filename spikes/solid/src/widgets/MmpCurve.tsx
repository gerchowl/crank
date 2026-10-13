import { createMemo, createSignal, Show } from "solid-js";
import type uPlot from "uplot";
import type { MmpCurve as Mmp } from "@neo/shared";
import { backend } from "../backend";
import { axis, tooltipPlugin, UPlot } from "../charts/UPlot";
import { fmtTimeAgo } from "../lib/format";
import { createPoll } from "../lib/primitives";
import type { WidgetProps } from "./common";
import { TableView } from "./TableView";

const watts = (_u: uPlot, v: number | null) => (v == null ? "—" : `${Math.round(v)} W`);
const TICKS = [1, 5, 15, 60, 300, 1200, 3600];

/** P09 — mean-maximal power: log x (1 s–60 min); today vs 90-day vs all-time. */
export function MmpCurve(props: WidgetProps & { data?: Mmp }) {
  const [table, setTable] = createSignal(false);
  const polled = createPoll(() => backend.mmp(), () => Math.min(props.widget.rateHz, 0.2), () => !props.data);
  const mmp = () => props.data ?? polled();
  const data = createMemo((): uPlot.AlignedData => {
    const m = mmp();
    if (!m) return [[], [], [], []];
    return [m.durationsS, m.today.map((v) => (v > 0 ? v : null)), m.best90d, m.allTime];
  });

  return (
    <div class="chart-widget">
      <div class="w-label">
        <span>
          Power-duration (MMP)
          <span class="legend-inline">
            <span class="chip-line" style={{ background: "var(--series-power)" }} /> today <span class="chip-dash solid" /> 90 d <span class="chip-dots muted-ink" /> all-time
          </span>
        </span>
        <span class="w-tools">
          <button class="mini" classList={{ on: table() }} onClick={() => setTable(!table())} title="table view">
            ▦
          </button>
        </span>
      </div>
      <Show
        when={!table()}
        fallback={
          <TableView
            caption="mean-maximal power (W)"
            head={["duration", "today", "90 d", "all-time"]}
            rows={(mmp()?.durationsS ?? []).map((d, i) => [fmtTimeAgo(d), Math.round(mmp()?.today[i] ?? 0) || "—", Math.round(mmp()?.best90d[i] ?? 0), Math.round(mmp()?.allTime[i] ?? 0)])}
          />
        }
      >
        <UPlot
          data={data}
          options={(t) => ({
            legend: { show: false },
            plugins: [tooltipPlugin((d) => fmtTimeAgo(d), (v) => `${Math.round(v)} W`)],
            cursor: { y: false, points: { size: 7 }, drag: { x: false, y: false } },
            padding: [8, 10, 0, 0],
            // eslint-disable-next-line @typescript-eslint/no-unsafe-enum-assignment -- uPlot's Distr is an ambient const enum (3 = log); not importable under isolatedModules
            scales: { x: { time: false, distr: 3, log: 10 }, y: { range: (_u, _lo, hi) => [0, Math.ceil((hi * 1.05) / 100) * 100] } },
            axes: [axis(t, { splits: () => TICKS, values: (_u, v) => v.map((s) => ((s as number | null) == null ? "" : fmtTimeAgo(s))), size: 22 }), axis(t, { size: 40 })],
            series: [
              { label: "duration", value: (_u: uPlot, v: number | null) => (v == null ? "—" : fmtTimeAgo(v)) },
              { label: "today", stroke: t.power, width: 2, value: watts },
              { label: "90 d", stroke: t.ink2, width: 1.5, dash: [5, 4], value: watts, points: { show: false } },
              { label: "all-time", stroke: t.muted, width: 1.5, dash: [1, 3], value: watts, points: { show: false } },
            ],
          })}
        />
      </Show>
    </div>
  );
}
