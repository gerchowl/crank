// SVG charts. They draw backend data as given (no physiology, DESIGN §10 #2). P9: axes with units,
// labelled marks, gaps never bridged (decimation keeps every null boundary), colour never alone.
import type { FlatBlock, HrRecoveryState, MmpCurve, Zone } from "@crank/shared";
import { zoneIndex } from "@crank/shared";
import { mmss } from "./format";

interface ProfileProps {
  blocks: FlatBlock[];
  zones: Zone[];
  height: number;
  fromS?: number;
  toS?: number;
  cursorS?: number;
  /** Highlight blocks whose path starts with this top-level index (editor selection / changed item). */
  selected?: number | null;
  /** A pending coach call: work blocks from `from` on are drawn dashed at their proposed height; applied changes (aiChanged) stay solid. */
  pending?: { from: number; pct: number } | undefined;
  onPick?: ((topIndex: number) => void) | undefined;
  /** Health cap: a labelled horizontal line no target may cross */
  capW?: number | undefined;
  label: string;
}

/** Workout profile (VIEWS P13): width = time, height = power, sequential zone ramp. */
export function Profile({ blocks, zones, height, fromS = 0, toS, cursorS, selected, pending, onPick, capW, label }: ProfileProps) {
  const end = toS ?? Math.max(1, ...blocks.map((b) => b.startS + b.durS));
  const span = Math.max(1, end - fromS);
  const top = Math.max(1, capW ?? 0, ...blocks.map((b) => Math.max(b.powerFrom, b.powerTo))) * 1.08;
  const x = (s: number) => ((s - fromS) / span) * 1000;
  const y = (w: number) => height - (w / top) * height;
  const poly = (b: FlatBlock, k = 1) => {
    const x0 = x(b.startS);
    const x1 = x(b.startS + b.durS);
    return `${x0},${height} ${x0},${y(b.powerFrom * k)} ${x1},${y(b.powerTo * k)} ${x1},${height}`;
  };
  const visible = (b: FlatBlock) => b.startS + b.durS > fromS && b.startS < end;
  return (
    <svg className="profile" viewBox={`0 0 1000 ${height}`} preserveAspectRatio="none" style={{ height }} role="img" aria-label={label}>
      <defs>
        <pattern id="hatch" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
          <line x1="0" y1="0" x2="0" y2="6" stroke="var(--ink-muted)" strokeWidth="2" />
        </pattern>
      </defs>
      {blocks.map((b, i) => {
        if (!visible(b)) return null;
        const z = zoneIndex(zones, (b.powerFrom + b.powerTo) / 2) + 1;
        const dim = cursorS !== undefined && b.startS + b.durS <= cursorS;
        const sel = selected !== undefined && selected !== null && b.path[0] === selected;
        const proposed = pending !== undefined && i >= pending.from && b.phase === "work";
        return (
          <g key={i} onClick={onPick ? () => onPick(b.path[0] ?? 0) : undefined} className={onPick ? "pickable" : undefined}>
            <polygon points={poly(b)} fill={`var(--zone-${z})`} opacity={dim ? 0.35 : 1} />
            {b.durRangeS && <polygon points={poly(b)} fill="url(#hatch)" opacity={0.35} />}
            {(b.aiChanged || sel) && <polygon points={poly(b)} fill="none" stroke={sel ? "var(--accent)" : "var(--ink-primary)"} strokeWidth={2.5} vectorEffect="non-scaling-stroke" />}
            {proposed && <polygon points={poly(b, 1 + pending.pct / 100)} fill="none" stroke="var(--status-info)" strokeWidth={3} strokeDasharray="7 4" vectorEffect="non-scaling-stroke" />}
          </g>
        );
      })}
      {cursorS !== undefined && <line x1={x(cursorS)} x2={x(cursorS)} y1={0} y2={height} stroke="var(--ink-primary)" strokeWidth={3} vectorEffect="non-scaling-stroke" />}
      {capW !== undefined && <line x1={0} x2={1000} y1={y(capW)} y2={y(capW)} className="cap-line" vectorEffect="non-scaling-stroke" />}
    </svg>
  );
}

// ---------- linear-time panels (VIEWS P05) ----------
type Vals = (number | null)[];
interface Series { values: Vals; cls: string; label: string; dashed?: boolean; area?: boolean }

/** Null-aware decimation: per bucket keep min and max, and break the line at any null in the bucket. */
function decimate(vals: Vals, buckets: number): { i: number; lo: number; hi: number }[][] {
  const step = Math.max(1, Math.ceil(vals.length / buckets));
  const runs: { i: number; lo: number; hi: number }[][] = [];
  let run: { i: number; lo: number; hi: number }[] = [];
  for (let i = 0; i < vals.length; i += step) {
    let lo = Infinity;
    let hi = -Infinity;
    let gap = false;
    for (let j = i; j < Math.min(vals.length, i + step); j++) {
      const v = vals[j];
      if (v === null || v === undefined) gap = true;
      else {
        lo = Math.min(lo, v);
        hi = Math.max(hi, v);
      }
    }
    if (Number.isFinite(lo)) run.push({ i, lo, hi });
    if (gap) {
      if (run.length) runs.push(run);
      run = [];
    }
  }
  if (run.length) runs.push(run);
  return runs;
}

/** Spans of seconds with no data in `vals` (for shading). */
export function gaps(vals: Vals): [number, number][] {
  const out: [number, number][] = [];
  let start = -1;
  vals.forEach((v, i) => {
    if (v === null && start < 0) start = i;
    if (v !== null && start >= 0) {
      out.push([start, i]);
      start = -1;
    }
  });
  if (start >= 0) out.push([start, vals.length]);
  return out;
}

const niceMax = (v: number) => {
  const p = 10 ** Math.floor(Math.log10(Math.max(1, v)));
  return ([1, 1.2, 1.5, 2, 2.5, 3, 4, 5, 6, 8, 10].find((m) => m * p >= v) ?? 10) * p;
};

export function TimeAxis({ n, fromS = 0 }: { n: number; fromS?: number }) {
  const span = Math.max(1, n);
  const every = span > 3600 ? 900 : span > 1500 ? 600 : span > 400 ? 120 : 30;
  const ticks = [];
  for (let s = Math.ceil(fromS / every) * every; s <= fromS + span; s += every) ticks.push(s);
  return (
    <div className="time-axis" aria-hidden>
      {ticks.map((s) => (
        <span key={s} style={{ left: `${((s - fromS) / span) * 100}%` }}>
          {mmss(s)}
        </span>
      ))}
    </div>
  );
}

/** One panel of a stacked linear-time chart. Panels share x by sharing `n` (seconds) and `fromS`. */
export function Panel({ series, height, unit, markers = [], markerNo, n, fromS = 0, lo: loIn = 0, zoom = false, between, betweenLabel }: { series: Series[]; height: number; unit: string; markers?: number[]; markerNo?: number[]; n: number; fromS?: number; lo?: number; zoom?: boolean; between?: [Vals, Vals]; betweenLabel?: string | undefined }) {
  const all = series.flatMap((s) => s.values.filter((v): v is number => v !== null));
  // zoom: the data range ± 15 %, so small changes (a −5 % target, a 9 bpm gap) are visible
  const lo = zoom ? Math.floor((Math.min(...all) * 0.85) / 10) * 10 : loIn;
  const hi = zoom ? Math.ceil((Math.max(...all) * 1.15) / 10) * 10 : niceMax(Math.max(...all, lo + 1) * 1.05);
  const X = (i: number) => (i / Math.max(1, n - 1)) * 1000;
  const Y = (v: number) => height - ((Math.max(lo, v) - lo) / (hi - lo)) * height;
  // gaps come from the measured series (the plan line never has gaps)
  const main = series.find((s) => s.cls.startsWith("series-")) ?? series.find((s) => !s.area);
  return (
    <div className="panel">
      <span className="axis-y hi">
        {hi} {unit}
      </span>
      <span className="axis-y lo">{lo}</span>
      <svg viewBox={`0 0 1000 ${height}`} preserveAspectRatio="none" style={{ height }} role="img" aria-label={`${series.map((s) => s.label).join(", ")} over the ride`}>
        <defs>
          <pattern id={`gaphatch-${unit}`} width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
            <line x1="0" y1="0" x2="0" y2="6" className="gap-line" strokeWidth="2" />
          </pattern>
        </defs>
        {/* gaps: a dashed outline box with a light tint — not the hatching, which means "variable length" on profiles */}
        {main && gaps(main.values).map(([a, b]) => <rect key={a} x={X(a)} width={Math.max(4, X(b) - X(a))} y={1} height={height - 2} className="gap" vectorEffect="non-scaling-stroke" />)}
        {between && (
          <polygon
            className="between"
            points={`${between[0].flatMap((v, i) => (v === null ? [] : [`${X(i)},${Y(v)}`])).join(" ")} ${[...between[1]].map((v, i) => [v, i] as const).reverse().flatMap(([v, i]) => (v === null ? [] : [`${X(i)},${Y(v)}`])).join(" ")}`}
          />
        )}
        {series.map((s) =>
          decimate(s.values, 900).map((run, k) =>
            s.area ? (
              <g key={`${s.cls}${k}`}>
                <polygon className={`area ${s.cls}`} points={`${X(run[0]?.i ?? 0)},${height} ${run.map((p) => `${X(p.i)},${Y(p.hi)}`).join(" ")} ${X(run.at(-1)?.i ?? 0)},${height}`} />
                {/* the area's edge carries the encoding (≥ 3:1); the fill is decoration */}
                <polyline className="area-edge" points={run.map((p) => `${X(p.i)},${Y(p.hi)}`).join(" ")} vectorEffect="non-scaling-stroke" />
              </g>
            ) : (
              <polyline
                key={`${s.cls}${k}`}
                className={`line ${s.cls}`}
                points={run.map((p) => `${X(p.i)},${Y((p.lo + p.hi) / 2)}`).join(" ")}
                strokeDasharray={s.dashed ? "5 4" : undefined}
                vectorEffect="non-scaling-stroke"
              />
            ),
          ),
        )}
        {markers.map((m) => (
          <line key={m} x1={X(m - fromS)} x2={X(m - fromS)} y1={0} y2={height} className="marker" vectorEffect="non-scaling-stroke" />
        ))}
      </svg>
      {betweenLabel && markers[0] !== undefined && (
        <span className="between-label" style={{ left: `${((markers[0] - fromS) / Math.max(1, n - 1)) * 100}%` }}>
          {betweenLabel}
        </span>
      )}
      {markers.map((m, i) => (
        <span key={m} className="marker-label" style={{ left: `${((m - fromS) / Math.max(1, n - 1)) * 100}%` }}>
          {markerNo?.[i] ?? i + 1}
        </span>
      ))}
    </div>
  );
}

/** HR recovery (VIEWS P12): observed (gaps kept), prediction with band, target, block window, live dot, time axis. */
export function Recovery({ r, height, live, predict = true }: { r: HrRecoveryState; height: number; live: boolean; predict?: boolean }) {
  const pred = predict ? (r.predicted ?? []) : [];
  const t0 = r.startedT;
  const t1 = Math.max(t0 + r.maxS, ...pred.map((p) => p.t));
  const obs = r.observed.filter((s): s is { t: number; v: number } => s.v !== null);
  const vals = [...obs.map((p) => p.v), ...pred.flatMap((p) => [p.lo, p.hi]), r.targetBpm];
  const lo = Math.floor((Math.min(...vals) - 4) / 10) * 10;
  const hi = Math.ceil((Math.max(...vals) + 4) / 10) * 10;
  const x = (t: number) => ((t - t0) / Math.max(1, t1 - t0)) * 1000;
  const y = (v: number) => height - ((v - lo) / (hi - lo)) * height;
  const runs: string[] = [];
  let cur = "";
  for (const s of r.observed) {
    if (s.v === null) {
      if (cur) runs.push(cur);
      cur = "";
    } else cur += `${x(s.t)},${y(s.v)} `;
  }
  if (cur) runs.push(cur);
  const band = pred.length ? `${pred.map((p) => `${x(p.t)},${y(p.hi)}`).join(" ")} ${[...pred].reverse().map((p) => `${x(p.t)},${y(p.lo)}`).join(" ")}` : "";
  const last = obs.at(-1);
  const minutes = Array.from({ length: Math.floor((t1 - t0) / 60) + 1 }, (_, i) => i);
  return (
    <div className="recovery-chart">
      <span className="axis-y hi">{hi}</span>
      <span className="axis-y lo">{lo} bpm</span>
      <svg viewBox={`0 0 1000 ${height}`} preserveAspectRatio="none" style={{ height, width: "100%" }} role="img" aria-label="Heart-rate recovery and prediction">
        <rect x={x(t0 + r.minS)} width={x(t0 + r.maxS) - x(t0 + r.minS)} y={0} height={height} className="window" />
        {band && <polygon points={band} className="band" />}
        {band && <polygon points={band} className="band-edge" vectorEffect="non-scaling-stroke" />}
        <line x1={0} x2={1000} y1={y(r.targetBpm)} y2={y(r.targetBpm)} className="target-line" vectorEffect="non-scaling-stroke" />
        {runs.map((pts, i) => (
          <polyline key={i} points={pts} className="line series-hr" vectorEffect="non-scaling-stroke" />
        ))}
        {pred.length > 0 && <polyline points={pred.map((p) => `${x(p.t)},${y(p.mean)}`).join(" ")} className="line series-hr" strokeDasharray="3 4" vectorEffect="non-scaling-stroke" />}
      </svg>
      {last && <i className={live ? "live-dot" : "live-dot stale"} style={{ left: `${x(last.t) / 10}%`, top: `${y(last.v)}px` }} />}
      <span className="target-tag" style={{ top: `${y(r.targetBpm)}px` }}>
        {r.targetBpm}
      </span>
      <div className="time-axis" aria-hidden>
        {minutes.map((m) => (
          <span key={m} style={{ left: `${x(t0 + m * 60) / 10}%` }}>
            {m}′
          </span>
        ))}
      </div>
    </div>
  );
}

/** Power-duration curve (VIEWS P09), 5 s – 60 min log x. Durations beyond the ride (today = 0) are omitted. */
export function PdCurve({ mmp, height }: { mmp: MmpCurve; height: number }) {
  const idx = mmp.durationsS.map((d, i) => [d, i] as const).filter(([d]) => d >= 5 && d <= 3600);
  const d0 = Math.log(5);
  const d1 = Math.log(3600);
  const lx = (s: number) => ((Math.log(s) - d0) / (d1 - d0)) * 1000;
  const top = niceMax(Math.max(...idx.map(([, i]) => Math.max(mmp.allTime[i] ?? 0, mmp.today[i] ?? 0))) * 1.05);
  const y = (w: number) => height - (w / top) * height;
  const line = (v: number[], cls: string, dash?: string) => (
    <polyline
      points={idx.flatMap(([d, i]) => ((v[i] ?? 0) > 0 ? [`${lx(d)},${y(v[i] ?? 0)}`] : [])).join(" ")}
      className={`line ${cls}`}
      strokeDasharray={dash}
      vectorEffect="non-scaling-stroke"
    />
  );
  const ticks: [number, string][] = [
    [5, "5 s"],
    [30, "30 s"],
    [60, "1 m"],
    [300, "5 m"],
    [1200, "20 m"],
    [3600, "60 m"],
  ];
  return (
    <div className="pd">
      <span className="axis-y hi">{top} W</span>
      <span className="axis-y mid" style={{ top: `${6 + height / 2}px` }}>
        {top / 2}
      </span>
      <span className="axis-y lo">0</span>
      <svg viewBox={`0 0 1000 ${height}`} preserveAspectRatio="none" style={{ height, width: "100%" }} role="img" aria-label="Power-duration curve">
        <line x1={0} x2={1000} y1={y(top / 2)} y2={y(top / 2)} className="grid" vectorEffect="non-scaling-stroke" />
        {line(mmp.allTime, "series-muted", "2 4")}
        {line(mmp.best90d, "series-muted")}
        {line(mmp.today, "series-power")}
      </svg>
      <div className="time-axis">
        {ticks.map(([s, l]) => (
          <span key={s} style={{ left: `${lx(s) / 10}%` }}>
            {l}
          </span>
        ))}
      </div>
    </div>
  );
}

/** Bars with axes (time at power). */
export function Bars({ bins, binWidth }: { bins: { lo: number; seconds: number }[]; binWidth: number }) {
  const max = niceMax(Math.max(60, ...bins.map((b) => b.seconds)));
  const last = (bins.at(-1)?.lo ?? 0) + binWidth;
  const step = last > 600 ? 200 : 100;
  const ticks = Array.from({ length: Math.floor(last / step) + 1 }, (_, i) => i * step);
  return (
    <div className="bars-chart">
      <span className="axis-y hi">{mmss(max)}</span>
      <span className="axis-y mid" style={{ top: "76px" }}>
        {mmss(max / 2)}
      </span>
      <span className="axis-y lo">0:00</span>
      <div className="bars" role="img" aria-label="Time at power">
        {bins.map((b) => (
          <i key={b.lo} style={{ height: `${(b.seconds / max) * 100}%` }} title={`${b.lo}–${b.lo + binWidth} W: ${mmss(b.seconds)}`} />
        ))}
      </div>
      <div className="time-axis">
        {ticks.map((w) => (
          <span key={w} style={{ left: `${(w / last) * 100}%` }}>
            {w} W
          </span>
        ))}
      </div>
    </div>
  );
}

/** Value ± sd over labelled dates (CP trend). */
export function Trend({ points, unit, height }: { points: { label: string; v: number; sd: number }[]; unit: string; height: number }) {
  const lo = Math.floor(Math.min(...points.map((p) => p.v - p.sd)) / 10) * 10;
  const hi = Math.ceil(Math.max(...points.map((p) => p.v + p.sd)) / 10) * 10;
  const y = (v: number) => height - ((v - lo) / (hi - lo)) * height;
  const x = (i: number) => (i / Math.max(1, points.length - 1)) * 1000;
  return (
    <div className="trend">
      <span className="axis-y hi">
        {hi} {unit}
      </span>
      <span className="axis-y lo">{lo}</span>
      <svg viewBox={`0 0 1000 ${height}`} preserveAspectRatio="none" style={{ height, width: "100%" }} role="img" aria-label={`Trend in ${unit} with ± 1 sd`}>
        <polygon className="band" points={`${points.map((p, i) => `${x(i)},${y(p.v + p.sd)}`).join(" ")} ${[...points].reverse().map((p, i) => `${x(points.length - 1 - i)},${y(p.v - p.sd)}`).join(" ")}`} />
        <polyline className="band-edge" points={points.map((p, i) => `${x(i)},${y(p.v + p.sd)}`).join(" ")} vectorEffect="non-scaling-stroke" />
        <polyline className="band-edge" points={points.map((p, i) => `${x(i)},${y(p.v - p.sd)}`).join(" ")} vectorEffect="non-scaling-stroke" />
        <polyline className="line series-power" points={points.map((p, i) => `${x(i)},${y(p.v)}`).join(" ")} vectorEffect="non-scaling-stroke" />
      </svg>
      <div className="time-axis">
        {points.map((p, i) =>
          i % 2 === 0 || i === points.length - 1 ? (
            <span key={p.label} style={{ left: `${x(i) / 10}%` }}>
              {p.label}
            </span>
          ) : null,
        )}
      </div>
    </div>
  );
}
