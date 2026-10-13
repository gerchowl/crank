import { type Accessor, createMemo, createSignal, For, Show } from "solid-js";
import type uPlot from "uplot";
import type { CoachNudge, FlatBlock, MetricId } from "@neo/shared";
import { backend } from "../backend";
import { axis, tooltipPlugin, UPlot } from "../charts/UPlot";
import { clock, zoneIdx } from "../lib/format";
import { type ThemeTokens, withAlpha, zoneColor } from "../theme/theme";
import { mean, type RideRecord } from "./streams";

interface PanelDef {
  metric: MetricId;
  label: string;
  unit: string;
  color: (t: ThemeTokens) => string;
  refs?: { v: number; label: string }[];
}

const PANELS: PanelDef[] = [
  { metric: "power", label: "Power", unit: "W", color: (t) => t.power },
  { metric: "hr", label: "Heart rate", unit: "bpm", color: (t) => t.hr },
  { metric: "cadence", label: "Cadence", unit: "rpm", color: (t) => t.cadence },
  {
    metric: "dfa_a1",
    label: "DFA α1",
    unit: "",
    color: (t) => t.dfa,
    refs: [
      { v: 0.75, label: "0.75 ≈ LT1" },
      { v: 0.5, label: "0.5 ≈ LT2" },
    ],
  },
  { metric: "wbal_pct", label: "W'bal", unit: "%", color: (t) => t.ink2 },
];
let seqNo = 0;

/** P05 — linear-time stacked panels sharing x (one crosshair), workout bands, coach ◆, select → stats, zoom. */
export function LinearPanels(props: { record: Accessor<RideRecord>; blocks: FlatBlock[]; nudges: readonly CoachNudge[] }) {
  const key = `p05-${++seqNo}`;
  const [sel, setSel] = createSignal<[number, number] | null>(null);
  const [zoom, setZoom] = createSignal<[number, number] | null>(null);
  let xs: number[] = [];

  const dataFor = (m: MetricId) => (): uPlot.AlignedData => {
    const r = props.record();
    if (xs.length > r.len) xs = [];
    for (let i = xs.length; i < r.len; i++) xs.push(i);
    const ys = r.series[m] ?? [];
    return [xs, ys.length === xs.length ? ys : ys.slice(0, xs.length)];
  };

  const stats = createMemo(() => {
    const s = sel();
    const r = props.record();
    if (!s) return null;
    return {
      from: s[0],
      to: s[1],
      power: mean(r.series["power"], s[0], s[1]),
      hr: mean(r.series["hr"], s[0], s[1]),
      cadence: mean(r.series["cadence"], s[0], s[1]),
    };
  });

  const options = (p: PanelDef, last: boolean) => (t: ThemeTokens): Omit<uPlot.Options, "width" | "height"> => {
    const blocks = props.blocks;
    const nudges = props.nudges;
    const zones = backend.zones.power;
    const hooks: uPlot.Hooks.Arrays = {
      setSelect: [
        (u) => {
          if (u.select.width < 4) return;
          const a = u.posToVal(u.select.left, "x");
          const b = u.posToVal(u.select.left + u.select.width, "x");
          setSel([Math.max(0, a), b]);
        },
      ],
    };
    if (p.metric === "power") {
      // workout profile as background bands (zone colour, low alpha)
      hooks.drawClear = [
        (u) => {
          const ctx = u.ctx;
          ctx.save();
          for (const b of blocks) {
            const x0 = u.valToPos(b.startS, "x", true);
            const x1 = u.valToPos(b.startS + b.durS, "x", true);
            const y = u.valToPos((b.powerFrom + b.powerTo) / 2, "y", true);
            ctx.fillStyle = withAlpha(zoneColor(t, zoneIdx(zones, (b.powerFrom + b.powerTo) / 2)), 0.22);
            ctx.fillRect(x0, y, Math.max(1, x1 - x0 - 1), u.bbox.top + u.bbox.height - y);
          }
          ctx.restore();
        },
      ];
    }
    hooks.draw = [
      (u) => {
        const ctx = u.ctx;
        ctx.save();
        // coach decisions as ◆ markers (power panel) / thin rules elsewhere
        for (const n of nudges) {
          const x = u.valToPos(n.t, "x", true);
          if (x < u.bbox.left || x > u.bbox.left + u.bbox.width) continue;
          ctx.strokeStyle = withAlpha(t.ink2, 0.5);
          ctx.setLineDash([2, 3]);
          ctx.beginPath();
          ctx.moveTo(x, u.bbox.top);
          ctx.lineTo(x, u.bbox.top + u.bbox.height);
          ctx.stroke();
          ctx.setLineDash([]);
          if (p.metric === "power") {
            ctx.fillStyle = t.ink;
            ctx.beginPath();
            const y = u.bbox.top + 6 * devicePixelRatio;
            const d = 5 * devicePixelRatio;
            ctx.moveTo(x, y - d);
            ctx.lineTo(x + d, y);
            ctx.lineTo(x, y + d);
            ctx.lineTo(x - d, y);
            ctx.fill();
          }
        }
        // reference lines (DFA α1 bands), labelled
        for (const r of p.refs ?? []) {
          const y = u.valToPos(r.v, "y", true);
          ctx.strokeStyle = t.target;
          ctx.setLineDash([6, 4]);
          ctx.beginPath();
          ctx.moveTo(u.bbox.left, y);
          ctx.lineTo(u.bbox.left + u.bbox.width, y);
          ctx.stroke();
          ctx.setLineDash([]);
          ctx.fillStyle = t.muted;
          ctx.font = `${10 * devicePixelRatio}px ${t.font}`;
          ctx.fillText(r.label, u.bbox.left + u.bbox.width - 70 * devicePixelRatio, y - 3 * devicePixelRatio);
        }
        ctx.restore();
      },
    ];
    return {
      legend: { show: false },
      plugins: [tooltipPlugin((x) => clock(x), (v) => `${p.metric === "dfa_a1" ? v.toFixed(2) : Math.round(v)} ${p.unit}`)],
      cursor: { sync: { key }, y: false, drag: { x: true, y: false, setScale: false } },
      select: { show: true, left: 0, top: 0, width: 0, height: 0 },
      padding: [4, 8, 0, 0],
      scales: { x: { time: false }, ...(p.refs ? { y: { range: [0, 1.6] } } : {}) },
      axes: [axis(t, { size: last ? 22 : 4, values: (_u, v) => (last ? v.map((s) => clock(s)) : v.map(() => "")) }), axis(t, { size: 42 })],
      series: [{}, { label: p.label, stroke: p.color(t), width: p.metric === "power" ? 1 : 1.5, points: { show: false } }],
      hooks,
    };
  };

  return (
    <div class="card p05">
      <div class="w-label">
        <span>
          Ride — stacked panels <span class="muted">drag to select an interval</span>
        </span>
        <span class="w-tools">
          <Show when={stats()}>
            {(s) => (
              <span class="sel-stats">
                {clock(s().from)}–{clock(s().to)} · <b>{Math.round(s().power ?? 0)} W</b> · <b>{Math.round(s().hr ?? 0)} bpm</b> · <b>{Math.round(s().cadence ?? 0)} rpm</b>
                <button class="mini" onClick={() => setZoom([s().from, s().to])}>
                  zoom
                </button>
                <button class="mini" onClick={() => setSel(null)}>
                  ✕
                </button>
              </span>
            )}
          </Show>
          <Show when={zoom()}>
            <button class="mini on" onClick={() => setZoom(null)}>
              reset zoom
            </button>
          </Show>
        </span>
      </div>
      <div class="p05-panels">
        <For each={PANELS}>
          {(p, i) => (
            <div class="p05-panel">
              <span class="p05-label">
                <span class="chip-line" style={{ background: p.metric === "wbal_pct" ? "var(--ink-secondary)" : `var(--series-${p.metric === "dfa_a1" ? "dfa" : p.metric})` }} />
                {p.label} <span class="muted">{p.unit}</span>
              </span>
              <UPlot data={dataFor(p.metric)} options={options(p, i() === PANELS.length - 1)} xRange={zoom} />
            </div>
          )}
        </For>
      </div>
    </div>
  );
}
