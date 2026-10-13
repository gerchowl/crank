import { useEffect, useMemo, useState } from "react";
import type uPlot from "uplot";
import { zoneIndex, type CoachNudge, type FlatBlock, type MetricId } from "@neo/shared";
import { backend } from "../backend";
import { useUPlot, type PlotOpts } from "../charts/useUPlot";
import { alpha, axisStyle } from "../charts/axes";
import { mmss } from "../lib/format";
import { useChartTheme, zoneColor, type ChartTheme } from "../theme/theme";
import { info, seriesColor } from "../ride/widgets/metrics";
import { mean, type RideRecord } from "./record";

type Range = [number, number] | null;
interface PanelDef { metric: MetricId; refs?: { v: number; label: string }[] }
const PANELS: PanelDef[] = [
  { metric: "power" },
  { metric: "hr" },
  { metric: "cadence" },
  { metric: "dfa_a1", refs: [{ v: 0.75, label: "≈ LT1 0.75" }, { v: 0.5, label: "≈ LT2 0.5" }] },
  { metric: "wbal_pct", refs: [{ v: 25, label: "floor" }] },
];

function panelOpts(t: ChartTheme, def: PanelDef, first: boolean, last: boolean, blocks: readonly FlatBlock[], nudges: readonly CoachNudge[], onSelect: (r: Range) => void): PlotOpts {
  const m = info(def.metric);
  const series: uPlot.Series[] = [
    { label: "t", value: (_u, v) => (Number.isFinite(v) ? mmss(v) : "—") },
    { label: m.label, stroke: seriesColor(t, def.metric), width: 1.5, points: { show: false }, value: (_u, v) => (Number.isFinite(v) ? `${def.metric === "dfa_a1" ? v.toFixed(2) : String(Math.round(v))} ${m.unit}` : "—") },
  ];
  if (def.metric === "power") series.push({ label: "target", stroke: t.target, width: 1.5, dash: [6, 4], points: { show: false }, value: (_u, v) => (Number.isFinite(v) ? `${String(Math.round(v))} W` : "—") });
  return {
    padding: [first ? 14 : 4, 8, 0, 0],
    legend: { show: true, live: true },
    cursor: { sync: { key: "review-p05" }, drag: { x: true, y: false, setScale: false }, points: { size: 6 } },
    select: { show: true, left: 0, top: 0, width: 0, height: 0 },
    scales: { x: { time: false }, y: { auto: true } },
    series,
    axes: [
      { ...axisStyle(t), size: last ? 24 : 2, values: (_u, s) => (last ? s.map((v) => mmss(v)) : s.map(() => "")) },
      { ...axisStyle(t), size: 44, label: m.unit || m.label, labelSize: 14, labelFont: `11px ${t.font}` },
    ],
    hooks: {
      setSelect: [
        (u) => {
          if (u.select.width < 4) return;
          const lo = u.posToVal(u.select.left, "x");
          const hi = u.posToVal(u.select.left + u.select.width, "x");
          u.setSelect({ left: 0, top: 0, width: 0, height: 0 }, false);
          onSelect([lo, hi]);
        },
      ],
      drawClear: [
        (u) => {
          // workout profile as background bands (zone ramp, low alpha)
          const { ctx } = u;
          const top = u.bbox.top;
          const h = u.bbox.height;
          ctx.save();
          for (const b of blocks) {
            const x0 = u.valToPos(b.startS, "x", true);
            const x1 = u.valToPos(b.startS + b.durS, "x", true);
            const z = zoneIndex(backend.zones.power, (b.powerFrom + b.powerTo) / 2);
            ctx.fillStyle = alpha(zoneColor(t, z), t.dark ? 0.16 : 0.2);
            ctx.fillRect(x0, top, Math.max(0, x1 - x0 - 1), h);
          }
          ctx.restore();
        },
      ],
      draw: [
        (u) => {
          const { ctx } = u;
          ctx.save();
          ctx.font = `${String(11 * devicePixelRatio)}px ${t.font}`;
          for (const r of def.refs ?? []) {
            const y = u.valToPos(r.v, "y", true);
            if (y < u.bbox.top || y > u.bbox.top + u.bbox.height) continue;
            ctx.strokeStyle = t.target;
            ctx.setLineDash([4, 4]);
            ctx.lineWidth = devicePixelRatio;
            ctx.beginPath();
            ctx.moveTo(u.bbox.left, y);
            ctx.lineTo(u.bbox.left + u.bbox.width, y);
            ctx.stroke();
            ctx.fillStyle = t.ink2;
            ctx.textAlign = "right";
            ctx.fillText(r.label, u.bbox.left + u.bbox.width - 4, y - 3);
          }
          ctx.setLineDash([2, 3]);
          for (const n of nudges) {
            const x = u.valToPos(n.t, "x", true);
            if (x < u.bbox.left || x > u.bbox.left + u.bbox.width) continue;
            ctx.strokeStyle = t.ink2;
            ctx.beginPath();
            ctx.moveTo(x, u.bbox.top);
            ctx.lineTo(x, u.bbox.top + u.bbox.height);
            ctx.stroke();
            if (first) {
              ctx.fillStyle = t.ink;
              ctx.textAlign = "center";
              ctx.fillText(n.status === "declined" || n.status === "expired" ? "◇" : "◆", x, u.bbox.top - 2);
            }
          }
          ctx.restore();
        },
      ],
    },
  };
}

function Panel(props: { def: PanelDef; first: boolean; last: boolean; data: uPlot.AlignedData; blocks: readonly FlatBlock[]; nudges: readonly CoachNudge[]; range: Range; onSelect: (r: Range) => void }) {
  const { def, first, last, data, blocks, nudges, range, onSelect } = props;
  const theme = useChartTheme();
  // onSelect is a state setter (stable), so it can live in the options without refs
  const opts = useMemo(() => panelOpts(theme, def, first, last, blocks, nudges, onSelect), [theme, def, first, last, blocks, nudges, onSelect]);
  const { el, setData, setXRange } = useUPlot(opts);
  // runs after the ref callback (re)created the chart, so a theme switch keeps data and zoom
  useEffect(() => {
    setData(data);
    setXRange(range);
  }, [data, range, opts, setData, setXRange]);
  return <div className={`plot-panel ${first ? "first" : ""}`} ref={el} />;
}

/** P05 linear time stacked panels for the whole ride + P14 DFA bands; P20 markers; brush → zoom + interval stats. */
export function RideTimeline({ rec, blocks, nudges }: { rec: RideRecord; blocks: readonly FlatBlock[]; nudges: readonly CoachNudge[] }) {
  const [range, setRange] = useState<Range>(null);
  const xs = useMemo(() => Array.from({ length: rec.toS }, (_, i) => i), [rec.toS]);
  const data = useMemo(
    () =>
      PANELS.map((p): uPlot.AlignedData =>
        p.metric === "power" ? [xs, rec.series["power"] ?? [], rec.series["target_power"] ?? []] : [xs, rec.series[p.metric] ?? []],
      ),
    [xs, rec],
  );
  const lo = Math.max(0, Math.floor(range?.[0] ?? 0));
  const hi = Math.min(rec.toS, Math.ceil(range?.[1] ?? rec.toS));
  const sl = (m: string) => (rec.series[m] ?? []).slice(lo, hi);
  const fmt = (v: number | null, d = 0) => (v === null ? "—" : v.toFixed(d));
  return (
    <section className="rv-card rv-timeline">
      <header className="rv-head">
        <h3>Ride · stacked panels</h3>
        <span className="muted small">drag to select an interval</span>
        <span className="spacer" />
        {range && (
          <span className="rv-sel">
            {mmss(lo)}–{mmss(hi)}: <b>{fmt(mean(sl("power")))} W</b> · {fmt(mean(sl("hr")))} bpm · {fmt(mean(sl("cadence")))} rpm{" "}
            <button type="button" className="btn small" onClick={() => { setRange(null); }}>Reset zoom</button>
          </span>
        )}
      </header>
      <div className="rv-stack">
        {PANELS.map((p, i) => (
          <Panel key={p.metric} def={p} first={i === 0} last={i === PANELS.length - 1} data={data[i] ?? [[]]} blocks={blocks} nudges={nudges} range={range} onSelect={setRange} />
        ))}
      </div>
    </section>
  );
}
