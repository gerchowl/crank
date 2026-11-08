// The recorded ride's traces on Review (DESIGN §5.5 item 4, §18.3; #529): power, heart rate and cadence stacked on one
// x axis of session time with time ticks (P05), each in its series colour with its word (§18.7 rule 11). The data path
// is the ride session's own history (session.ts): the window keeps the traces of the ride it rode until the next load,
// and the core has no command that returns a stored ride's samples, so a ride opened from the list has none (#107 is
// Review Evidence; #557's `ride_series` would bring them back). Drawn as Trace.tsx draws: one vertex per second of
// session time (`perSecond`), a missing second is a gap and never bridged, no smoothing. Heart rate is on 80–200 bpm
// (§5.5: a 0-based axis flattens the differences that matter). Ranges are found by a loop, never by spreading a
// ride-length array into `Math.min`/`Math.max` (a 3 h ride at 4 Hz is 43 200 points; #557).
import { useMemo } from "react";
import { perSecond } from "../ride/Trace";
import type { Point, Series } from "../ride/session";
import { formatDuration } from "./Workouts";

const W = 1000;
const H = 100;
/** Power's headroom over its highest sample, and the lowest top it gets (Trace.tsx draws power the same way). */
const HEADROOM = 1.1;
const POWER_FLOOR = 100;
/** The heart-rate axis of §5.5. */
const HR_AXIS: [number, number] = [80, 200];
/** Cadence's margin around its samples, rpm. */
const CAD_MARGIN = 10;

/** The lowest and the highest value of `points`, `null` for none: one pass, no spread. */
export function extent(points: readonly Point[]): [number, number] | null {
  if (points.length === 0) return null;
  let lo = Infinity;
  let hi = -Infinity;
  for (const p of points) {
    if (p.v < lo) lo = p.v;
    if (p.v > hi) hi = p.v;
  }
  return [lo, hi];
}

/** The y axis of a series: power from 0, heart rate fixed, cadence around its samples. */
export function axis(
  series: Series,
  points: readonly Point[],
): [number, number] {
  const e = extent(points);
  if (series === "hr") return HR_AXIS;
  if (series === "power")
    return [0, Math.round(HEADROOM * Math.max(POWER_FLOOR, e?.[1] ?? 0))];
  return e === null
    ? [0, 120]
    : [
        Math.max(0, Math.floor(e[0] - CAD_MARGIN)),
        Math.ceil(e[1] + CAD_MARGIN),
      ];
}

/** Tick steps, s: the first that gives at most six ticks over the ride. */
const STEPS = [60, 120, 300, 600, 900, 1800, 3600, 7200];
/** The time ticks inside the axis, none in its last 15 %: the end's own label sits there. */
export function ticks(xMax: number): number[] {
  const step = STEPS.find((s) => xMax / s <= 6) ?? STEPS[STEPS.length - 1];
  const out: number[] = [];
  for (let t = step; t < xMax * 0.85; t += step) out.push(t);
  return out;
}

/** The end of the x axis: the recorded duration, or the last sample if one came later. */
export function axisEnd(
  durationS: number | null,
  series: Record<Series, readonly Point[]>,
): number {
  let end = durationS ?? 0;
  for (const s of Object.values(series)) {
    const last = s[s.length - 1];
    if (last && last.t > end) end = last.t;
  }
  return Math.max(end, 1);
}

const LABEL: Record<Series, [string, string]> = {
  power: ["Power", "W"],
  hr: ["Heart rate", "bpm"],
  cadence: ["Cadence", "rpm"],
};

function Panel({
  series,
  points,
  xMax,
}: {
  series: Series;
  points: readonly Point[];
  xMax: number;
}) {
  const [label, unit] = LABEL[series];
  const [lo, hi] = axis(series, points);
  // Built once per history: Review's history no longer grows, and a long ride has 10 800 vertices a series.
  const lines = useMemo(() => {
    const x = (s: number) => ((s / xMax) * W).toFixed(1);
    const y = (v: number) => (H - ((v - lo) / (hi - lo)) * H).toFixed(1);
    return perSecond(points).map((run) =>
      run.map((p) => `${x(p.t)},${y(p.v)}`).join(" "),
    );
  }, [points, xMax, lo, hi]);
  return (
    <div className="review-trace" data-series={series}>
      <span className="review-trace-label">
        <span data-series-label>{label}</span> · {unit} ·{" "}
        <span className="num">
          {lo}–{hi}
        </span>
      </span>
      {points.length === 0 ? (
        <p className="review-trace-none">
          No {label.toLowerCase()} was recorded.
        </p>
      ) : (
        // eslint-disable-next-line no-restricted-syntax -- a data graphic, not an icon (DESIGN §18.7 rule 4 is about icons).
        <svg
          data-graphic
          role="img"
          aria-label={`${label} over the ride, ${lo} to ${hi} ${unit}`}
          viewBox={`0 0 ${W} ${H}`}
          preserveAspectRatio="none"
        >
          {lines.map((points, i) => (
            <polyline
              key={i}
              data-testid={`review-${series}`}
              className="review-trace-line"
              points={points}
              vectorEffect="non-scaling-stroke"
            />
          ))}
        </svg>
      )}
    </div>
  );
}

/** The three traces of a recorded ride over `durationS`, with the time axis under them. */
export function RideTraces({
  series,
  durationS,
}: {
  series: Record<Series, readonly Point[]>;
  durationS: number | null;
}) {
  const xMax = axisEnd(durationS, series);
  return (
    <div className="review-traces">
      <Panel series="power" points={series.power} xMax={xMax} />
      <Panel series="hr" points={series.hr} xMax={xMax} />
      <Panel series="cadence" points={series.cadence} xMax={xMax} />
      <div className="review-axis" aria-hidden>
        <span className="num" style={{ left: "0%" }}>
          0:00
        </span>
        {ticks(xMax).map((t) => (
          <span
            key={t}
            className="num review-tick"
            style={{ left: `${(t / xMax) * 100}%` }}
          >
            {formatDuration(t)}
          </span>
        ))}
        <span className="num review-axis-end">{formatDuration(xMax)}</span>
      </div>
    </div>
  );
}
