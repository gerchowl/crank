// SVG charts. They draw backend data as given (no physiology, DESIGN §10 #2); decimation is for
// drawing only. Colours come from CSS tokens (spec/THEMES.md) via classes / var().
import type { FlatBlock, HrRecoveryState, MmpCurve, Zone } from "@crank/shared";
import { zoneIndex } from "@crank/shared";

interface ProfileProps {
  blocks: FlatBlock[];
  zones: Zone[];
  height: number;
  fromS?: number;
  toS?: number;
  cursorS?: number;
  ghost?: FlatBlock[] | undefined;
  /** Highlight blocks whose path starts with this top-level index (editor selection). */
  selected?: number | null;
  onPick?: ((topIndex: number) => void) | undefined;
  label: string;
}

/** Workout profile (VIEWS P13): width = time, height = power, sequential zone ramp. */
export function Profile({ blocks, zones, height, fromS = 0, toS, cursorS, ghost, selected, onPick, label }: ProfileProps) {
  const end = toS ?? Math.max(1, ...blocks.map((b) => b.startS + b.durS));
  const span = Math.max(1, end - fromS);
  const top = Math.max(1, ...blocks.map((b) => Math.max(b.powerFrom, b.powerTo)), ...(ghost ?? []).map((b) => Math.max(b.powerFrom, b.powerTo))) * 1.08;
  const x = (s: number) => ((s - fromS) / span) * 1000;
  const y = (w: number) => height - (w / top) * height;
  const poly = (b: FlatBlock) => {
    const x0 = x(b.startS);
    const x1 = x(b.startS + b.durS);
    return `${x0},${height} ${x0},${y(b.powerFrom)} ${x1},${y(b.powerTo)} ${x1},${height}`;
  };
  const visible = (b: FlatBlock) => b.startS + b.durS > fromS && b.startS < end;
  return (
    <svg className="profile" viewBox={`0 0 1000 ${height}`} preserveAspectRatio="none" style={{ height }} role="img" aria-label={label}>
      <defs>
        <pattern id="hatch" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
          <line x1="0" y1="0" x2="0" y2="6" stroke="var(--ink-muted)" strokeWidth="2" />
        </pattern>
      </defs>
      {blocks.filter(visible).map((b, i) => {
        const z = zoneIndex(zones, (b.powerFrom + b.powerTo) / 2) + 1;
        const dim = cursorS !== undefined && b.startS + b.durS <= cursorS;
        const sel = selected !== undefined && selected !== null && b.path[0] === selected;
        return (
          <g key={i} onClick={onPick ? () => onPick(b.path[0] ?? 0) : undefined} className={onPick ? "pickable" : undefined}>
            <polygon points={poly(b)} fill={`var(--zone-${z})`} opacity={dim ? 0.35 : 1} />
            {b.durRangeS && <polygon points={poly(b)} fill="url(#hatch)" opacity={0.35} />}
            {(b.aiChanged || sel) && <polygon points={poly(b)} fill="none" stroke={sel ? "var(--accent)" : "var(--ink-primary)"} strokeWidth={2} vectorEffect="non-scaling-stroke" />}
          </g>
        );
      })}
      {ghost?.filter(visible).map((b, i) => (
        <polygon key={`g${i}`} points={poly(b)} fill="none" stroke="var(--ink-primary)" strokeDasharray="4 3" strokeWidth={1.5} vectorEffect="non-scaling-stroke" />
      ))}
      {cursorS !== undefined && <line x1={x(cursorS)} x2={x(cursorS)} y1={0} y2={height} stroke="var(--ink-primary)" strokeWidth={3} vectorEffect="non-scaling-stroke" />}
    </svg>
  );
}

interface Series { values: (number | null)[]; cls: string; dashed?: boolean }
/** One panel of a stacked linear-time chart (VIEWS P05). Panels share x by sharing `n`. */
export function Panel({ series, height, unit, markers = [], n }: { series: Series[]; height: number; unit: string; markers?: number[]; n: number }) {
  const all = series.flatMap((s) => s.values.filter((v): v is number => v !== null));
  const lo = Math.min(...all, 0);
  const hi = Math.max(...all, 1) * 1.05;
  const step = Math.max(1, Math.ceil(n / 900));
  const path = (vals: (number | null)[]) => {
    let d = "";
    let pen = false;
    for (let i = 0; i < vals.length; i += step) {
      const v = vals[i];
      if (v === null || v === undefined) {
        pen = false;
        continue;
      }
      d += `${pen ? "L" : "M"}${((i / Math.max(1, n - 1)) * 1000).toFixed(1)},${(height - ((v - lo) / (hi - lo)) * height).toFixed(1)}`;
      pen = true;
    }
    return d;
  };
  return (
    <div className="panel">
      <span className="panel-unit">{unit}</span>
      <svg viewBox={`0 0 1000 ${height}`} preserveAspectRatio="none" style={{ height }} role="img" aria-label={`${unit} over the ride`}>
        {markers.map((m) => (
          <line key={m} x1={(m / Math.max(1, n - 1)) * 1000} x2={(m / Math.max(1, n - 1)) * 1000} y1={0} y2={height} className="marker" vectorEffect="non-scaling-stroke" />
        ))}
        {series.map((s) => (
          <path key={s.cls} d={path(s.values)} className={`line ${s.cls}`} strokeDasharray={s.dashed ? "5 4" : undefined} vectorEffect="non-scaling-stroke" />
        ))}
      </svg>
    </div>
  );
}

/** HR recovery (VIEWS P12): observed, fitted prediction with band, target line. */
export function Recovery({ r, height }: { r: HrRecoveryState; height: number }) {
  const pts = r.observed.filter((s): s is { t: number; v: number } => s.v !== null);
  const pred = r.predicted ?? [];
  const t0 = r.startedT;
  const t1 = Math.max(t0 + r.minS, ...pred.map((p) => p.t), ...pts.map((p) => p.t));
  const vals = [...pts.map((p) => p.v), ...pred.flatMap((p) => [p.lo, p.hi]), r.targetBpm];
  const lo = Math.min(...vals) - 4;
  const hi = Math.max(...vals) + 4;
  const x = (t: number) => ((t - t0) / Math.max(1, t1 - t0)) * 1000;
  const y = (v: number) => height - ((v - lo) / (hi - lo)) * height;
  const band = pred.length ? `${pred.map((p) => `${x(p.t)},${y(p.hi)}`).join(" ")} ${[...pred].reverse().map((p) => `${x(p.t)},${y(p.lo)}`).join(" ")}` : "";
  return (
    <svg viewBox={`0 0 1000 ${height}`} preserveAspectRatio="none" style={{ height, width: "100%" }} role="img" aria-label="Heart-rate recovery and prediction">
      {band && <polygon points={band} className="band" />}
      <line x1={0} x2={1000} y1={y(r.targetBpm)} y2={y(r.targetBpm)} className="target-line" vectorEffect="non-scaling-stroke" />
      <polyline points={pts.map((p) => `${x(p.t)},${y(p.v)}`).join(" ")} className="line series-hr" vectorEffect="non-scaling-stroke" />
      {pred.length > 0 && <polyline points={pred.map((p) => `${x(p.t)},${y(p.mean)}`).join(" ")} className="line series-hr" strokeDasharray="3 4" vectorEffect="non-scaling-stroke" />}
    </svg>
  );
}

/** Power-duration curve (VIEWS P09), log x. Durations beyond the ride (today = 0) are omitted. */
export function PdCurve({ mmp, height }: { mmp: MmpCurve; height: number }) {
  const d = mmp.durationsS;
  const lx = (s: number) => ((Math.log(s) - Math.log(d[0] ?? 1)) / (Math.log(d.at(-1) ?? 3600) - Math.log(d[0] ?? 1))) * 1000;
  const top = Math.max(...mmp.allTime, ...mmp.today) * 1.05;
  const y = (w: number) => height - (w / top) * height;
  const line = (v: number[], cls: string, dash?: string) => (
    <polyline
      points={v.flatMap((w, i) => (w > 0 ? [`${lx(d[i] ?? 1)},${y(w)}`] : [])).join(" ")}
      className={`line ${cls}`}
      strokeDasharray={dash}
      vectorEffect="non-scaling-stroke"
    />
  );
  const ticks: [number, string][] = [
    [1, "1 s"],
    [10, "10 s"],
    [60, "1 m"],
    [300, "5 m"],
    [1200, "20 m"],
    [3600, "60 m"],
  ];
  return (
    <div className="pd">
      <span className="pd-y">{Math.round(top)} W</span>
      <svg viewBox={`0 0 1000 ${height}`} preserveAspectRatio="none" style={{ height, width: "100%" }} role="img" aria-label="Power-duration curve">
        {line(mmp.allTime, "series-muted", "2 4")}
        {line(mmp.best90d, "series-muted")}
        {line(mmp.today, "series-power")}
      </svg>
      <div className="pd-x">
        {ticks.map(([s, l]) => (
          <span key={s} style={{ left: `${lx(s) / 10}%` }}>
            {l}
          </span>
        ))}
      </div>
    </div>
  );
}
